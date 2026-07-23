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

/// The local git identity's member key when it holds an open invitation.
/// `None` when that e-mail is not a member, or is one that already enrolled.
/// This is what lets the project window prompt for an OTP after a local repo
/// is picked, without the user knowing they were invited.
pub fn pending_for_local_identity(root: &Path) -> Result<Option<String>, JoyError> {
    let email = crate::vcs::default_vcs().user_email()?;
    let project = store::load_project(root)?;
    let member_key =
        crate::privacy::member_key_for_email(&project, &email).unwrap_or_else(|| email.clone());
    Ok(is_pending(&project, &member_key).then_some(member_key))
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
    let member_key =
        crate::privacy::member_key_for_email(&project, &email).unwrap_or_else(|| email.clone());

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
