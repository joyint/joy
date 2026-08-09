// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! First-time enrollment: redeeming the invitation OTP a manage member
//! issued with `joy project member add`. Print-free like [`super::login`], so
//! the CLI, the desktop app and the platform share ONE implementation instead
//! of each orchestrating the same steps over the primitives.
//!
//! Invitations are a project-level concept, not a platform feature: the same
//! member map carries them whether the repo sits on a device or behind the
//! platform. What differs is only where the passphrase is turned into key
//! material, so the chosen passphrase never reaches [`apply_enrollment`]:
//! local callers derive seed and wraps themselves, the browser derives them
//! in-tab and hands over only the wraps. That split is what lets one function
//! serve a local repo and the platform alike.

use std::path::Path;

use crate::auth::{attestation, generate_salt, seed as seed_mod, session, IdentityKeypair};
use crate::error::JoyError;
use crate::model::project::Project;
use crate::store;
use crate::vcs::Vcs;

/// The identity material a redeemer derived from their chosen passphrase,
/// hex-encoded exactly as it is stored on the member.
pub struct EnrollmentMaterial {
    pub verify_key: String,
    pub kdf_nonce: String,
    pub seed_wrap_passphrase: String,
    pub seed_wrap_recovery: String,
}

/// What a completed local redemption produced. The recovery key is shown
/// once and never stored in plaintext.
pub struct EnrollmentOutcome {
    pub keypair: IdentityKeypair,
    pub recovery_key: seed_mod::RecoveryKey,
    /// The member-map key that enrolled (e-mail in open mode, opaque id in
    /// anonymous mode).
    pub member_key: String,
}

/// Whether `member_key` holds an unredeemed invitation: a verifier is on
/// file and setup is not complete. This is the posture the platform reports
/// as `pending`, computed the same way for a local repo.
pub fn is_pending(project: &Project, member_key: &str) -> bool {
    let Some(m) = project.member_by_key(member_key) else {
        return false;
    };
    let enrolled =
        m.verify_key.is_some() && m.kdf_nonce.is_some() && m.seed_wrap_passphrase.is_some();
    m.enrollment_verifier.is_some() && !enrolled
}

/// What the enrolling member presents about their invitation.
pub enum Proof<'a> {
    /// An invitation is on file and is proven with this one-time password.
    Otp(&'a str),
    /// No invitation: the founder, or a member a manager added without one.
    /// Refused when an invitation IS on file, so an invited slot can never be
    /// claimed without its OTP.
    FirstContact,
}

/// The member a redemption belongs to (JOY-0257-FC): the OTP is the
/// identity proof during enrolment, so the member whose PENDING
/// invitation it proves wins — the invited address in the manager's head
/// and the redeemer's git/account address need not agree (forge alias
/// addresses, JP-00BF-94). Only members with an open invitation are
/// scanned; when no verifier matches, the address resolves as before, so
/// every existing error text (already enrolled, no pending invitation)
/// stays word-for-word.
pub fn member_for_redemption(project: &Project, email: &str, otp: &str) -> Option<String> {
    let proven = project
        .members()
        .filter(|(_, m)| m.verify_key.is_none())
        .filter_map(|(key, m)| m.enrollment_verifier.as_deref().map(|v| (key, v)))
        .find(|(_, verifier)| crate::auth::otp::verify_otp(otp, verifier).unwrap_or(false))
        .map(|(key, _)| key.clone());
    proven.or_else(|| crate::privacy::member_key_for_email(project, email))
}

/// Check the invitation posture and write the material onto the member,
/// clearing the invitation when one was redeemed. Mutates the project in
/// memory; persisting it is the caller's job (the platform and the CLI's
/// anonymous mode write through their own paths).
///
/// The pairing of `proof` with what is on file is the security boundary: the
/// attestation signs e-mail, capabilities and the enrollment verifier, but NOT
/// the verify key, so nothing downstream would notice a member who set their
/// own key without ever proving the OTP. This is the one place that refuses it.
pub fn apply_enrollment(
    project: &mut Project,
    member_key: &str,
    proof: Proof<'_>,
    material: EnrollmentMaterial,
) -> Result<(), JoyError> {
    let (stored, already_enrolled) = {
        let member = project.member_by_key(member_key).ok_or_else(|| {
            JoyError::AuthFailed(format!(
                "{member_key} is not a registered project member. \
                 A member with manage capability must add you first."
            ))
        })?;
        (
            member.enrollment_verifier.clone(),
            member.verify_key.is_some(),
        )
    };
    if already_enrolled {
        return Err(JoyError::AuthFailed(format!(
            "{member_key} already completed setup."
        )));
    }
    let redeemed = match (stored, proof) {
        (Some(stored), Proof::Otp(otp)) => {
            if !crate::auth::otp::verify_otp(otp, &stored)? {
                return Err(JoyError::AuthFailed("incorrect OTP".into()));
            }
            true
        }
        (Some(_), Proof::FirstContact) => {
            return Err(JoyError::AuthFailed(format!(
                "{member_key} was invited: the one-time password is required. \
                 Redeem it instead of setting up a fresh identity."
            )));
        }
        (None, Proof::Otp(_)) => {
            return Err(JoyError::AuthFailed(format!(
                "no pending invitation for {member_key}. Either setup is already done \
                 or the member was added without an OTP."
            )));
        }
        (None, Proof::FirstContact) => false,
    };

    let m = project
        .member_by_key_mut(member_key)
        .expect("member resolved above");
    m.verify_key = Some(material.verify_key);
    m.kdf_nonce = Some(material.kdf_nonce);
    m.seed_wrap_passphrase = Some(material.seed_wrap_passphrase);
    m.seed_wrap_recovery = Some(material.seed_wrap_recovery);
    if redeemed {
        // The invitation is spent; `is_pending` now reports false and the
        // attestation's verifier check falls through to its post-redemption arm.
        m.enrollment_verifier = None;
    }
    Ok(())
}

/// The founder's member key while exactly one member is still unattested
/// (the solo founder, before the chain closes). JOY-00FD-93.
pub fn founder_needing_reverse_attestation(project: &Project) -> Option<String> {
    let mut unattested: Vec<&String> = project
        .members()
        .filter(|(_, m)| m.attestation.is_none())
        .map(|(key, _)| key)
        .collect();
    (unattested.len() == 1).then(|| unattested.remove(0).clone())
}

/// Close the attestation chain on first join: while the founder is the only
/// unattested member, the redeemer reverse-attests them. Verification needs
/// no manage capability, only a key that verifies the signature, so any
/// redeemer can close it (JOY-00FD-93).
pub fn reverse_attest_founder(project: &mut Project, redeemer: &str, keypair: &IdentityKeypair) {
    let Some(founder) = founder_needing_reverse_attestation(project) else {
        return;
    };
    if founder == redeemer {
        return;
    }
    let Some(founder_member) = project.member_by_key(&founder).cloned() else {
        return;
    };
    let signed_fields = attestation::signed_fields_for(
        &founder,
        &founder_member.capabilities,
        founder_member.enrollment_verifier.as_deref(),
    );
    let att = attestation::sign_attestation(redeemer, keypair, signed_fields);
    if let Some(m) = project.member_by_key_mut(&founder) {
        m.attestation = Some(att);
    }
}

/// The whole local redemption: derive the identity from the chosen
/// passphrase, apply it, persist, and open a session. The CLI and the desktop
/// app both call this; only the presentation of the recovery key differs.
pub fn redeem_with_passphrase(
    root: &Path,
    otp: &str,
    passphrase: &str,
) -> Result<EnrollmentOutcome, JoyError> {
    crate::auth::validate_passphrase(passphrase)?;

    let email = crate::vcs::default_vcs().user_email()?;
    let project_path = store::joy_dir(root).join(store::PROJECT_FILE);
    let mut project = store::load_project(root)?;
    // The OTP finds its member (JOY-0257-FC); the raw address only
    // survives as the fallthrough so apply_enrollment can answer with
    // its precise refusal texts.
    let member_key = member_for_redemption(&project, &email, otp).unwrap_or_else(|| email.clone());

    // Wrapped-seed onboarding (ADR-039): a fresh random seed, wrapped under
    // both the passphrase KEK and the recovery KEK.
    let salt = generate_salt();
    let seed = seed_mod::Seed::generate();
    let recovery_key = seed_mod::RecoveryKey::generate();
    let seed_wrap_passphrase = seed_mod::wrap_seed_with_passphrase(&seed, passphrase, &salt)?;
    let seed_wrap_recovery = seed_mod::wrap_seed_with_recovery(&seed, &recovery_key, &salt)?;
    let keypair = IdentityKeypair::from_seed(seed.as_bytes());

    apply_enrollment(
        &mut project,
        &member_key,
        Proof::Otp(otp),
        EnrollmentMaterial {
            verify_key: keypair.public_key().to_hex(),
            kdf_nonce: salt.to_hex(),
            seed_wrap_passphrase,
            seed_wrap_recovery,
        },
    )?;
    reverse_attest_founder(&mut project, &member_key, &keypair);

    store::write_yaml_preserve(&project_path, &project)?;
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    crate::git_ops::auto_git_add(root, &[&rel]);

    // Establish the new member's first session.
    let project_id = session::project_id(root)?;
    let token = session::create_session(&keypair, &member_key, &project_id, None);
    session::save_session(&project_id, &token)?;

    Ok(EnrollmentOutcome {
        keypair,
        recovery_key,
        member_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::otp;
    use crate::model::project::{Member, MemberCapabilities, Project};

    fn material() -> EnrollmentMaterial {
        EnrollmentMaterial {
            verify_key: "aa".into(),
            kdf_nonce: "bb".into(),
            seed_wrap_passphrase: "cc".into(),
            seed_wrap_recovery: "dd".into(),
        }
    }

    fn project_with(verifier: Option<String>) -> (Project, String) {
        let mut project = Project::new("T".into(), Some("T".into()));
        let mut m = Member::new(MemberCapabilities::All);
        m.enrollment_verifier = verifier;
        let key = "alice@example.com".to_string();
        project.register_member(&key, m).unwrap();
        (project, key)
    }

    #[test]
    fn first_contact_is_refused_while_an_invitation_is_pending() {
        // The security boundary: an invited slot must not be claimable with a
        // self-chosen key. The attestation does not cover the verify key, so
        // this is the only thing that catches it.
        let otp = otp::generate_otp();
        let (mut project, key) = project_with(Some(otp::hash_otp(&otp).unwrap()));
        let err =
            apply_enrollment(&mut project, &key, Proof::FirstContact, material()).unwrap_err();
        assert!(err.to_string().contains("one-time password is required"));
        assert!(is_pending(&project, &key));
        assert!(project.member_by_key(&key).unwrap().verify_key.is_none());
    }

    #[test]
    fn the_right_otp_enrolls_and_clears_the_invitation() {
        let otp = otp::generate_otp();
        let (mut project, key) = project_with(Some(otp::hash_otp(&otp).unwrap()));
        apply_enrollment(&mut project, &key, Proof::Otp(&otp), material()).unwrap();
        let m = project.member_by_key(&key).unwrap();
        assert_eq!(m.verify_key.as_deref(), Some("aa"));
        assert!(m.enrollment_verifier.is_none());
        assert!(!is_pending(&project, &key));
    }

    #[test]
    fn a_wrong_otp_is_refused() {
        let otp = otp::generate_otp();
        let (mut project, key) = project_with(Some(otp::hash_otp(&otp).unwrap()));
        let err = apply_enrollment(&mut project, &key, Proof::Otp("WRNG-OTPX-0000"), material())
            .unwrap_err();
        assert!(err.to_string().contains("incorrect OTP"));
        assert!(project.member_by_key(&key).unwrap().verify_key.is_none());
    }

    #[test]
    fn first_contact_enrolls_a_founder_without_an_invitation() {
        let (mut project, key) = project_with(None);
        apply_enrollment(&mut project, &key, Proof::FirstContact, material()).unwrap();
        assert_eq!(
            project.member_by_key(&key).unwrap().verify_key.as_deref(),
            Some("aa")
        );
    }

    #[test]
    fn an_otp_without_an_invitation_is_refused() {
        let (mut project, key) = project_with(None);
        let err =
            apply_enrollment(&mut project, &key, Proof::Otp("anything"), material()).unwrap_err();
        assert!(err.to_string().contains("no pending invitation"));
    }

    #[test]
    fn the_otp_finds_its_member_whatever_the_redeemers_address_says() {
        // JOY-0257-FC / JP-00BF-94: invited as alice@example.com, but the
        // redeemer's git config carries a forge alias. The OTP is the
        // identity proof, so redemption still lands on the invited slot.
        let otp = otp::generate_otp();
        let (project, key) = project_with(Some(otp::hash_otp(&otp).unwrap()));
        let alias = "12345+alice@users.noreply.github.com";
        assert_eq!(
            member_for_redemption(&project, alias, &otp).as_deref(),
            Some(key.as_str())
        );
        // a matching address still resolves like before
        assert_eq!(
            member_for_redemption(&project, &key, &otp).as_deref(),
            Some(key.as_str())
        );
    }

    #[test]
    fn a_wrong_otp_and_a_strange_address_resolve_to_nobody() {
        let otp = otp::generate_otp();
        let (project, _key) = project_with(Some(otp::hash_otp(&otp).unwrap()));
        assert_eq!(
            member_for_redemption(&project, "stranger@example.com", "WRNG-OTPX-0000"),
            None
        );
    }

    #[test]
    fn an_enrolled_member_is_never_matched_by_otp_scan() {
        // Only PENDING invitations are scanned: an already enrolled member
        // (verify_key set) must not be reachable through a stale verifier.
        let otp = otp::generate_otp();
        let (mut project, key) = project_with(Some(otp::hash_otp(&otp).unwrap()));
        apply_enrollment(&mut project, &key, Proof::Otp(&otp), material()).unwrap();
        // enrolment cleared the verifier; even if it survived, the
        // verify_key filter alone must exclude the member
        project.member_by_key_mut(&key).unwrap().enrollment_verifier =
            Some(otp::hash_otp(&otp).unwrap());
        assert_eq!(
            member_for_redemption(&project, "stranger@example.com", &otp),
            None
        );
    }
}
