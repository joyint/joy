// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Per-member attestation signing and verification.
//!
//! Each non-founder member in `project.yaml` carries an attestation signed
//! by a manage-capable member. Verification is purely local against
//! `project.yaml`; the attester's public key is read from the same file.
//!
//! Signed subset (see `AttestationSignedFields`): `email`, `capabilities`,
//! `otp_hash`. `public_key` is intentionally excluded so a member's
//! passphrase change does not break the attestation. Once a member has
//! redeemed their OTP and `otp_hash` has been cleared, verification
//! ignores `signed_fields.otp_hash` (the historical value is retained in
//! the attestation for audit).

use chrono::Utc;

use super::sign;
use crate::error::JoyError;
use crate::model::project::{Attestation, AttestationSignedFields, Member, MemberCapabilities};

/// Produce an attestation over `signed_fields` using the given attester
/// identity keypair.
pub fn sign_attestation(
    attester_email: &str,
    attester_keypair: &sign::IdentityKeypair,
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
    otp_hash: Option<&str>,
) -> AttestationSignedFields {
    AttestationSignedFields {
        email: email.to_string(),
        capabilities: capabilities.clone(),
        otp_hash: otp_hash.map(|s| s.to_string()),
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
/// 4. `signed_fields.otp_hash` matches the member's current `otp_hash`,
///    unless the member's `otp_hash` is `None` (post-redemption state).
pub fn verify_attestation(
    attestation: &Attestation,
    attester_public_key: &sign::PublicKey,
    member_email: &str,
    member: &Member,
) -> Result<(), JoyError> {
    let sig_bytes = hex::decode(&attestation.signature)
        .map_err(|e| JoyError::AuthFailed(format!("attestation signature is not hex: {e}")))?;
    let canonical = attestation.signed_fields.canonical_bytes();
    attester_public_key
        .verify(&canonical, &sig_bytes)
        .map_err(|_| JoyError::AuthFailed("attestation signature does not verify".into()))?;

    if attestation.signed_fields.email != member_email {
        return Err(JoyError::AuthFailed(
            "attestation email does not match member".into(),
        ));
    }
    if attestation.signed_fields.capabilities != member.capabilities {
        return Err(JoyError::AuthFailed(
            "attestation capabilities do not match member".into(),
        ));
    }
    // otp_hash match is required only while the member still has one.
    // Post-redemption the stored otp_hash is cleared; the attestation's
    // historical value is accepted.
    if let Some(current) = &member.otp_hash {
        if attestation.signed_fields.otp_hash.as_deref() != Some(current.as_str()) {
            return Err(JoyError::AuthFailed(
                "attestation otp_hash does not match member".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::project::{CapabilityConfig, MemberCapabilities};

    fn make_kp() -> sign::IdentityKeypair {
        sign::IdentityKeypair::from_random()
    }

    fn fresh_member(caps: MemberCapabilities, otp: Option<String>) -> Member {
        let mut m = Member::new(caps);
        m.otp_hash = otp;
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
    fn verify_accepts_cleared_otp_hash_post_redemption() {
        let kp = make_kp();
        let pk = kp.public_key();
        let fields = signed_fields_for(
            "alice@example.com",
            &MemberCapabilities::All,
            Some("abcd".into()),
        );
        let att = sign_attestation("horst@example.com", &kp, fields);
        // otp_hash cleared after redemption - should still verify.
        let member = fresh_member(MemberCapabilities::All, None);
        verify_attestation(&att, &pk, "alice@example.com", &member).unwrap();
    }

    #[test]
    fn verify_fails_on_email_mismatch() {
        let kp = make_kp();
        let pk = kp.public_key();
        let fields = signed_fields_for("alice@example.com", &MemberCapabilities::All, None);
        let att = sign_attestation("horst@example.com", &kp, fields);
        let member = fresh_member(MemberCapabilities::All, None);
        let err = verify_attestation(&att, &pk, "bob@example.com", &member).unwrap_err();
        assert!(matches!(err, JoyError::AuthFailed(msg) if msg.contains("email")));
    }

    #[test]
    fn verify_fails_on_otp_hash_mismatch_before_redemption() {
        let kp = make_kp();
        let pk = kp.public_key();
        let fields = signed_fields_for(
            "alice@example.com",
            &MemberCapabilities::All,
            Some("AAAA".into()),
        );
        let att = sign_attestation("horst@example.com", &kp, fields);
        let member = fresh_member(MemberCapabilities::All, Some("BBBB".into()));
        let err = verify_attestation(&att, &pk, "alice@example.com", &member).unwrap_err();
        assert!(matches!(err, JoyError::AuthFailed(msg) if msg.contains("otp_hash")));
    }
}
