// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! The passphrase login flow (JOY-01EA): everything `joy auth` does for a
//! registered wrapped-seed member, print-free, so the CLI and the desktop
//! app share ONE implementation (the app never shells out to the CLI).
//! The legacy-schema migration (no seed_wrap_*) and its recovery-key
//! printout stay in the CLI; this function refuses legacy entries with a
//! clear error instead.

use std::path::Path;

use crate::auth::{attestation, seed as seed_mod, session, IdentityKeypair, PublicKey, Salt};
use crate::error::JoyError;
use crate::model::project::{Attestation, Member, PrivacyMode, Project};
use crate::store;

/// What a successful login produced.
pub struct LoginOutcome {
    pub keypair: IdentityKeypair,
    /// The 32-byte identity seed (callers may unlock zones with it).
    pub seed: [u8; 32],
    /// The member-map key the session was created for (e-mail in open
    /// mode, opaque id in anonymous mode).
    pub member_key: String,
    /// Files opportunistically re-encrypted during login (ADR-040).
    pub relocked: usize,
    /// Whether the pre-feature auto-seal ran (JOY-0101-78).
    pub sealed: bool,
}

/// Authenticate `email` with `passphrase`: verify the identity, enforce
/// the attestation posture, create and persist the 24h session, cache the
/// anonymous members zone key, and opportunistically re-lock plaintext
/// zone files. Mirrors the CLI's `joy auth` for wrapped-seed members.
pub fn login(root: &Path, email: &str, passphrase: &str) -> Result<LoginOutcome, JoyError> {
    let project = store::load_project(root)?;
    let member_key =
        crate::privacy::member_key_for_email(&project, email).unwrap_or_else(|| email.to_string());
    let member = project.member_by_key(&member_key).ok_or_else(|| {
        JoyError::AuthFailed(format!(
            "{email} is not a registered project member. Run `joy project member add {email}`."
        ))
    })?;

    let public_key_hex = member.verify_key.as_ref().ok_or_else(|| {
        JoyError::AuthFailed(format!(
            "Authentication not initialized for {email}. Run `joy auth init`."
        ))
    })?;
    let salt_hex = member
        .kdf_nonce
        .as_ref()
        .ok_or_else(|| JoyError::AuthFailed(format!("No salt found for {email}.")))?;
    let public_key = PublicKey::from_hex(public_key_hex)?;
    let salt = Salt::from_hex(salt_hex)?;

    let wrap_hex = member.seed_wrap_passphrase.as_deref().ok_or_else(|| {
        JoyError::AuthFailed(format!(
            "{email} still uses the legacy auth schema; run `joy auth` in a terminal once to migrate."
        ))
    })?;
    let seed = seed_mod::unwrap_seed_with_passphrase(wrap_hex, passphrase, &salt)?;
    let keypair = IdentityKeypair::from_seed(seed.as_bytes());
    if keypair.public_key() != public_key {
        return Err(JoyError::AuthFailed("incorrect passphrase".into()));
    }

    // JOY-0101-78: silent auto-seal for pre-feature projects.
    let sealed_project = maybe_auto_seal(root, &project, &member_key, &keypair)?;
    let view = sealed_project.as_ref().unwrap_or(&project);
    let member = view
        .member_by_key(&member_key)
        .expect("member survived sealing");

    // JOY-0100-DA: attestation posture before establishing a session.
    if let Some(att) = member.attestation.as_ref() {
        verify_member_attestation(view, email, member, att)?;
    } else if attestation::founder_must_be_attested(view) {
        return Err(JoyError::AuthFailed(format!(
            "{email} has no attestation and the project has multiple members. \
             The entry appears to have been tampered with. Ask a manage member \
             to remove and re-add {email}."
        )));
    }

    let project_id = session::project_id(root)?;
    let mut token = session::create_session(&keypair, &member_key, &project_id, None);
    token.members_zone_key = cached_members_zone_key(view, &member_key, seed.as_bytes());
    session::save_session(&project_id, &token)?;

    let relocked = relock_unlocked_files(root, view, email, seed.as_bytes());

    Ok(LoginOutcome {
        seed: *seed.as_bytes(),
        keypair,
        member_key,
        relocked,
        sealed: sealed_project.is_some(),
    })
}

/// In anonymous mode, the hex-encoded members.yaml zone key for
/// `member_key`, cached in the session (ADR-042).
pub fn cached_members_zone_key(
    project: &Project,
    member_key: &str,
    seed: &[u8; 32],
) -> Option<String> {
    if project.privacy_mode() != PrivacyMode::Anonymous {
        return None;
    }
    let wrap = project.member_by_key(member_key)?.members_wrap.as_deref()?;
    let zk = crate::crypt::unwrap_for_member(wrap, crate::members_file::MEMBERS_ZONE, seed).ok()?;
    Some(hex::encode(zk.as_bytes()))
}

/// Verify the attestation against the attester's public key with the
/// CLI-aligned, user-facing error wording.
pub fn verify_member_attestation(
    project: &Project,
    email: &str,
    member: &Member,
    att: &Attestation,
) -> Result<(), JoyError> {
    let attester_entry = project.member_by_key(att.attester.id()).ok_or_else(|| {
        JoyError::AuthFailed(format!(
            "attestation for {email} names attester {} but that member is not registered. \
             Ask a manage member to remove and re-add {email}.",
            att.attester
        ))
    })?;
    let attester_pubkey_hex = attester_entry.verify_key.as_ref().ok_or_else(|| {
        JoyError::AuthFailed(format!(
            "attestation for {email} is signed by {} but that member has no public key. \
             Ask a manage member to remove and re-add {email}.",
            att.attester
        ))
    })?;
    let attester_pubkey = PublicKey::from_hex(attester_pubkey_hex)?;
    attestation::verify_attestation(att, &attester_pubkey, email, member).map_err(|e| {
        JoyError::AuthFailed(format!(
            "attestation for {email} is not valid ({e}). The entry appears to have been \
             tampered with. Ask a manage member to remove and re-add {email}."
        ))
    })
}

/// JOY-0101-78: if no member anywhere carries an attestation yet, treat
/// the current state as legitimate and sign attestations for every other
/// member with the acting member's keypair. Runs at most once, silent.
pub fn maybe_auto_seal(
    root: &Path,
    project: &Project,
    acting_email: &str,
    acting_keypair: &IdentityKeypair,
) -> Result<Option<Project>, JoyError> {
    let has_any_attestation = project.member_values().any(|m| m.attestation.is_some());
    if has_any_attestation || project.member_count() < 2 {
        return Ok(None);
    }

    let project_path = store::joy_dir(root).join(store::PROJECT_FILE);
    let mut sealed = store::read_project(&project_path)?;

    let targets: Vec<String> = sealed
        .member_keys()
        .filter(|email| email.as_str() != acting_email)
        .cloned()
        .collect();
    for target_email in targets {
        let target = sealed.member_by_key(&target_email).cloned().unwrap();
        let signed_fields = attestation::signed_fields_for(
            &target_email,
            &target.capabilities,
            target.enrollment_verifier.as_deref(),
        );
        let att = attestation::sign_attestation(acting_email, acting_keypair, signed_fields);
        sealed.member_by_key_mut(&target_email).unwrap().attestation = Some(att);
    }

    store::write_yaml_preserve(&project_path, &sealed)?;
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    crate::git_ops::auto_git_add(root, &[&rel]);

    Ok(Some(sealed))
}

/// ADR-040 opportunistic re-lock: any plaintext file under a zone this
/// member holds a wrap for gets re-encrypted. Best-effort and silent;
/// returns the count.
pub fn relock_unlocked_files(
    root: &Path,
    project: &Project,
    email: &str,
    seed: &[u8; 32],
) -> usize {
    let Some(member) = project.member_by_email(email) else {
        return 0;
    };
    let mut relocked = 0;
    for (zone, wrap_hex) in &member.crypt_wraps {
        let Ok(zone_key) = crate::crypt::unwrap_for_member(wrap_hex, zone, seed) else {
            continue;
        };
        let Some(zone_cfg) = project.crypt.zones.get(zone) else {
            continue;
        };
        for pattern in &zone_cfg.paths {
            relock_path(root, &zone_key, zone, pattern, &mut relocked);
        }
    }
    relocked
}

fn relock_path(
    root: &Path,
    zone_key: &crate::crypt::ZoneKey,
    zone: &str,
    pattern: &str,
    relocked: &mut usize,
) {
    let abs = root.join(pattern);
    if abs.is_file() {
        if relock_file(&abs, zone_key, zone) {
            *relocked += 1;
        }
    } else if abs.is_dir() {
        relock_dir(&abs, zone_key, zone, relocked);
    }
}

fn relock_dir(dir: &Path, zone_key: &crate::crypt::ZoneKey, zone: &str, relocked: &mut usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            relock_dir(&p, zone_key, zone, relocked);
        } else if p.is_file() && relock_file(&p, zone_key, zone) {
            *relocked += 1;
        }
    }
}

fn relock_file(path: &Path, zone_key: &crate::crypt::ZoneKey, zone: &str) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    if crate::crypt::looks_like_blob(&bytes) {
        return false;
    }
    let blob = crate::crypt::encrypt_blob(zone, zone_key, &bytes);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("relock"),
        std::process::id()
    ));
    if std::fs::write(&tmp, &blob).is_err() {
        return false;
    }
    if std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    true
}
