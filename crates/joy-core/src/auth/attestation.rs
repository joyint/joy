// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Per-member attestation signing and verification.
//!
//! Each non-founder member in `project.yaml` carries an attestation signed
//! by a manage-capable member. Verification is purely local against
//! `project.yaml`; the attester's public key is read from the same file.
//!
//! Signed subset (see `AttestationSignedFields`): `email`, `capabilities`,
//! `enrollment_verifier`. `verify_key` is intentionally excluded so a member's
//! passphrase change does not break the attestation. Once a member has
//! redeemed their OTP and `enrollment_verifier` has been cleared, verification
//! ignores `signed_fields.enrollment_verifier` (the historical value is retained in
//! the attestation for audit).

use chrono::Utc;

use super::{IdentityKeypair, PublicKey};
use crate::error::JoyError;
use crate::model::project::{Attestation, AttestationSignedFields, Member, MemberCapabilities};

/// Produce an attestation over `signed_fields` using the given attester
/// identity keypair.
pub fn sign_attestation(
    attester_email: &str,
    attester_keypair: &IdentityKeypair,
    signed_fields: AttestationSignedFields,
) -> Attestation {
    let bytes = signed_fields.canonical_bytes();
    let signature = attester_keypair.sign(&bytes);
    Attestation {
        attester: attester_email.to_string(),
        signed_fields,
        signed_at: Utc::now(),
        signature: hex::encode(signature),
    }
}

/// Build the signed-fields snapshot for a target member.
pub fn signed_fields_for(
    email: &str,
    capabilities: &MemberCapabilities,
    enrollment_verifier: Option<&str>,
) -> AttestationSignedFields {
    AttestationSignedFields {
        email: email.to_string(),
        capabilities: capabilities.clone(),
        enrollment_verifier: enrollment_verifier.map(|s| s.to_string()),
    }
}

/// Verify a member's attestation against its attester's public key and
/// the member's current state.
///
/// Checks:
/// 1. Signature verifies against `attester_public_key` over
///    `attestation.signed_fields`.
/// 2. `signed_fields.email` matches `member_email`.
/// 3. `signed_fields.capabilities` matches the member's current
///    capabilities.
/// 4. `signed_fields.enrollment_verifier` matches the member's current `enrollment_verifier`,
///    unless the member's `enrollment_verifier` is `None` (post-redemption state).
pub fn verify_attestation(
    attestation: &Attestation,
    attester_public_key: &PublicKey,
    member_email: &str,
    member: &Member,
) -> Result<(), JoyError> {
    let sig_bytes = hex::decode(&attestation.signature)
        .map_err(|e| JoyError::AuthFailed(format!("attestation signature is not hex: {e}")))?;

    // Recompute the signed bytes with the authoritative e-mail supplied by the
    // caller (the concept's `email_for`: the project.yaml key in open mode, the
    // decrypted members.yaml in anonymous mode), not the value stored on disk.
    // In anonymous mode (ADR-042) `signed_fields.email` on disk is an opaque-id
    // placeholder so no e-mail sits in project.yaml; the signature stays frozen
    // over the original e-mail and still verifies here because `email_for`
    // yields the bit-identical address. This single cryptographic check also
    // subsumes the old `signed_fields.email == member_email` comparison: a
    // signature that verifies over `member_email` proves the attester signed for
    // exactly this member.
    let mut signed = attestation.signed_fields.clone();
    signed.email = member_email.to_string();
    let canonical = signed.canonical_bytes();
    attester_public_key
        .verify(&canonical, &sig_bytes)
        .map_err(|_| JoyError::AuthFailed("attestation signature does not verify".into()))?;

    if attestation.signed_fields.capabilities != member.capabilities {
        return Err(JoyError::AuthFailed(
            "attestation capabilities do not match member".into(),
        ));
    }
    // enrollment_verifier match is required only while the member still has one.
    // Post-redemption the stored value is cleared; the attestation's
    // historical value is accepted.
    if let Some(current) = &member.enrollment_verifier {
        if attestation.signed_fields.enrollment_verifier.as_deref() != Some(current.as_str()) {
            return Err(JoyError::AuthFailed(
                "attestation enrollment_verifier does not match member".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::project::{CapabilityConfig, MemberCapabilities};

    fn make_kp() -> IdentityKeypair {
        IdentityKeypair::from_random()
    }

    fn fresh_member(caps: MemberCapabilities, otp: Option<String>) -> Member {
        let mut m = Member::new(caps);
        m.enrollment_verifier = otp;
        m
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let kp = make_kp();
        let pk = kp.public_key();
        let fields = signed_fields_for(
            "alice@example.com",
            &MemberCapabilities::All,
            Some("deadbeef"),
        );
        let att = sign_attestation("horst@example.com", &kp, fields);
        let member = fresh_member(MemberCapabilities::All, Some("deadbeef".into()));
        verify_attestation(&att, &pk, "alice@example.com", &member).unwrap();
    }

    #[test]
    fn verify_fails_on_tampered_capability() {
        let kp = make_kp();
        let pk = kp.public_key();
        let fields = signed_fields_for("alice@example.com", &MemberCapabilities::All, None);
        let att = sign_attestation("horst@example.com", &kp, fields);

        let mut caps = std::collections::BTreeMap::new();
        caps.insert(
            crate::model::item::Capability::Implement,
            CapabilityConfig::default(),
        );
        let member = fresh_member(MemberCapabilities::Specific(caps), None);

        let err = verify_attestation(&att, &pk, "alice@example.com", &member).unwrap_err();
        assert!(matches!(err, JoyError::AuthFailed(msg) if msg.contains("capabilities")));
    }

    #[test]
    fn verify_fails_on_tampered_signature() {
        let kp = make_kp();
        let pk = kp.public_key();
        let fields = signed_fields_for("alice@example.com", &MemberCapabilities::All, None);
        let mut att = sign_attestation("horst@example.com", &kp, fields);
        // Flip one hex digit in the signature.
        let mut bytes: Vec<char> = att.signature.chars().collect();
        bytes[0] = if bytes[0] == '0' { '1' } else { '0' };
        att.signature = bytes.into_iter().collect();

        let member = fresh_member(MemberCapabilities::All, None);
        let err = verify_attestation(&att, &pk, "alice@example.com", &member).unwrap_err();
        assert!(matches!(err, JoyError::AuthFailed(msg) if msg.contains("signature")));
    }

    #[test]
    fn verify_accepts_cleared_enrollment_verifier_post_redemption() {
        let kp = make_kp();
        let pk = kp.public_key();
        let fields = signed_fields_for("alice@example.com", &MemberCapabilities::All, Some("abcd"));
        let att = sign_attestation("horst@example.com", &kp, fields);
        // enrollment_verifier cleared after redemption - should still verify.
        let member = fresh_member(MemberCapabilities::All, None);
        verify_attestation(&att, &pk, "alice@example.com", &member).unwrap();
    }

    #[test]
    fn verify_survives_anonymous_id_placeholder() {
        // Open mode: the founder attests alice over her real e-mail.
        let kp = make_kp();
        let pk = kp.public_key();
        let fields = signed_fields_for("alice@example.com", &MemberCapabilities::All, None);
        let mut att = sign_attestation("horst@example.com", &kp, fields);

        // Switching to anonymous (ADR-042) replaces the plaintext e-mail in the
        // stored attestation (and the attester) with opaque ids; the signature
        // is deliberately NOT recomputed (frozen bytes).
        att.signed_fields.email = "m-alice".to_string();
        att.attester = "m-horst".to_string();

        // Verification injects the authoritative e-mail from email_for and still
        // passes: the signed bytes are reconstructed over the real address, which
        // is bit-identical to what was signed. This is the case that previously
        // broke (canonical bytes were taken from the rewritten id field).
        let member = fresh_member(MemberCapabilities::All, None);
        verify_attestation(&att, &pk, "alice@example.com", &member).unwrap();

        // And a wrong e-mail still fails, so the binding is intact.
        assert!(verify_attestation(&att, &pk, "mallory@example.com", &member).is_err());
    }

    #[test]
    fn verify_fails_on_email_mismatch() {
        let kp = make_kp();
        let pk = kp.public_key();
        let fields = signed_fields_for("alice@example.com", &MemberCapabilities::All, None);
        let att = sign_attestation("horst@example.com", &kp, fields);
        let member = fresh_member(MemberCapabilities::All, None);
        // The e-mail now enters the verification cryptographically: canonical
        // bytes are recomputed from the authoritative `member_email` (email_for),
        // so verifying for a different address fails the signature check itself
        // rather than a separate string compare. This is the stronger binding
        // that lets anonymous mode store an id placeholder for the e-mail on disk.
        let err = verify_attestation(&att, &pk, "bob@example.com", &member).unwrap_err();
        assert!(matches!(err, JoyError::AuthFailed(msg) if msg.contains("signature")));
    }

    #[test]
    fn verify_fails_on_enrollment_verifier_mismatch_before_redemption() {
        let kp = make_kp();
        let pk = kp.public_key();
        let fields = signed_fields_for("alice@example.com", &MemberCapabilities::All, Some("AAAA"));
        let att = sign_attestation("horst@example.com", &kp, fields);
        let member = fresh_member(MemberCapabilities::All, Some("BBBB".into()));
        let err = verify_attestation(&att, &pk, "alice@example.com", &member).unwrap_err();
        assert!(matches!(err, JoyError::AuthFailed(msg) if msg.contains("enrollment_verifier")));
    }
}
