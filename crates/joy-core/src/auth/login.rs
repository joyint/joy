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
use crate::model::project::{is_ai_member, Attestation, Member, PrivacyMode, Project};
use crate::store;

/// What a successful login produced.
pub struct LoginOutcome {
    pub keypair: IdentityKeypair,
    /// The 32-byte identity seed (callers may unlock zones with it).
    pub seed: [u8; 32],
    /// The member-map key the session was created for (e-mail in open
    /// mode, opaque id in anonymous mode).
    pub member_key: String,
    /// Who just authenticated, as a person reads it. In open mode that is
    /// the address the caller logged in with, which is what they typed.
    /// In anonymous mode the caller is usually holding the opaque `m-`
    /// id, because that is what the member pin answers with (D3.9), and
    /// telling somebody they are an opaque id is the one thing ADR-042
    /// asks every output not to do: there this is the address out of
    /// members.yaml, which this very login opened on its way past the
    /// attestation check, and a login that cannot read it fails instead
    /// of answering with the id. Never empty, and never an opaque id.
    pub address: String,
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
    // A miss consults the project's forge plugin (JOY-0253-8A): a forge
    // alias address still finds its member.
    let member_key = crate::privacy::member_key_for_email_or_forge(&project, root, email, None)
        .unwrap_or_else(|| email.to_string());
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
    finish_login(
        root,
        project,
        member_key,
        email,
        seed,
        &public_key,
        "incorrect passphrase",
    )
}

/// Authenticate with an ALREADY unlocked seed (the app's OS-keystore
/// remember, JAPP-0026): everything `login` does after the passphrase
/// unwrap — identity check, attestation posture, session, relock. The
/// seed must still derive the member's registered verify key.
pub fn login_with_seed(
    root: &Path,
    email: &str,
    seed_bytes: &[u8; 32],
) -> Result<LoginOutcome, JoyError> {
    let project = store::load_project(root)?;
    // A miss consults the project's forge plugin (JOY-0253-8A): a forge
    // alias address still finds its member.
    let member_key = crate::privacy::member_key_for_email_or_forge(&project, root, email, None)
        .unwrap_or_else(|| email.to_string());
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
    let public_key = PublicKey::from_hex(public_key_hex)?;
    let seed = seed_mod::Seed::from_bytes(*seed_bytes);
    finish_login(
        root,
        project,
        member_key,
        email,
        seed,
        &public_key,
        "the stored seed no longer matches the registered identity",
    )
}

/// The shared back half of both login flows: identity check against the
/// registered verify key, auto-seal, attestation posture, session
/// creation, opportunistic re-lock.
fn finish_login(
    root: &Path,
    project: Project,
    member_key: String,
    email: &str,
    seed: seed_mod::Seed,
    public_key: &PublicKey,
    mismatch_error: &str,
) -> Result<LoginOutcome, JoyError> {
    let keypair = IdentityKeypair::from_seed(seed.as_bytes());
    if keypair.public_key() != *public_key {
        return Err(JoyError::AuthFailed(mismatch_error.into()));
    }

    // JOY-0101-78: silent auto-seal for pre-feature projects.
    let sealed_project = maybe_auto_seal(root, &project, &member_key, &keypair)?;
    let view = sealed_project.as_ref().unwrap_or(&project);
    let member = view
        .member_by_key(&member_key)
        .expect("member survived sealing");

    // JOY-0100-DA: attestation posture before establishing a session.
    // The attestation binds the member's CANONICAL identity: in open mode
    // that is the member key itself. The raw login address may legally
    // differ (a forge alias resolved to its member, JOY-0253-8A) and must
    // not fail the binding check. Anonymous mode signs the ADDRESS, which
    // the opaque member key is not, so it is read out of members.yaml
    // rather than taken from whatever the caller came in holding.
    let attested_id = match view.privacy_mode() {
        PrivacyMode::Open => member_key.clone(),
        // An AI member keeps its synthetic key through the switch to
        // anonymous mode: it gets no members.yaml row, because there is
        // no person behind it to keep out of a committed file, and the
        // key is what an attestation over it signs, exactly as in open
        // mode.
        _ if is_ai_member(&member_key) => member_key.clone(),
        // A person, in anonymous mode: only members.yaml can say who
        // they are, and if it cannot, this says so. The fallback that
        // used to stand here handed the opaque id to the attestation
        // check, which no attestation signs, so a missing members.yaml
        // came back as "the entry appears to have been tampered with"
        // and sent a person looking at their own entry.
        _ => anonymous_attested_address(root, view, &member_key, seed.as_bytes())
            .ok_or_else(|| JoyError::AnonymousMemberUnnamed(member_key.clone()))?,
    };
    // What the person is told they just authenticated as. Open mode keeps
    // saying the address they came in with; anonymous mode says the one
    // the id resolves to, and never the id.
    let address = match view.privacy_mode() {
        PrivacyMode::Open => email.to_string(),
        _ => attested_id.clone(),
    };
    if let Some(att) = member.attestation.as_ref() {
        verify_member_attestation(view, &attested_id, member, att)?;
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
    token.chat_seed = Some(hex::encode(seed.as_bytes()));
    session::save_session(&project_id, &token)?;

    // Authenticating is a person saying who they are on this device, so
    // it is one of the moments that pins them (D3.9): every command
    // afterwards knows the member without asking git config, and removing
    // `user.email` changes nothing. The desktop app comes through here
    // too, so its login pins exactly as the CLI's does.
    crate::identity::pin_acting_member(root, view, &member_key);

    let relocked = relock_unlocked_files(root, view, &member_key, seed.as_bytes());

    Ok(LoginOutcome {
        seed: *seed.as_bytes(),
        keypair,
        member_key,
        address,
        relocked,
        sealed: sealed_project.is_some(),
    })
}

/// In anonymous mode, the members.yaml zone key for `member_key`, opened
/// with that member's own seed (ADR-042).
fn members_zone_key(
    project: &Project,
    member_key: &str,
    seed: &[u8; 32],
) -> Option<joy_crypt::zone::ZoneKey> {
    if project.privacy_mode() != PrivacyMode::Anonymous {
        return None;
    }
    let wrap = project.member_by_key(member_key)?.members_wrap.as_deref()?;
    joy_crypt::zone::unwrap_for_member(wrap, crate::members_file::MEMBERS_ZONE, seed).ok()
}

/// In anonymous mode, the hex-encoded members.yaml zone key for
/// `member_key`, cached in the session (ADR-042).
pub fn cached_members_zone_key(
    project: &Project,
    member_key: &str,
    seed: &[u8; 32],
) -> Option<String> {
    members_zone_key(project, member_key, seed).map(|zk| hex::encode(zk.as_bytes()))
}

/// The address an anonymous project's attestation for `member_key` was
/// signed over: the member's own e-mail, out of the encrypted
/// members.yaml, opened with the seed this login just derived.
///
/// An attestation never signs the opaque id (the id is this project's own
/// invention and says nothing about the person), so the id cannot answer
/// the binding check. Before package J11 the address arrived with the
/// caller, because `joy auth` resolved its member from `git config
/// user.email`. Now it arrives as the member this device pinned, which IS
/// the opaque id, and every returning member of an anonymous project was
/// told their own entry looked tampered with unless they typed `--user
/// <address>` again. D3.9 promises the opposite: naming yourself once
/// settles it, and the device remembers.
///
/// `None` when the members file cannot be opened (a member without a
/// wrap, a missing or stale file). The caller has nothing to fall back
/// to then: the identifier it was given is the opaque id in exactly the
/// case this exists for, so it raises
/// [`JoyError::AnonymousMemberUnnamed`] rather than answer with an id
/// that no attestation signs and that ADR-042 shows nobody.
fn anonymous_attested_address(
    root: &Path,
    project: &Project,
    member_key: &str,
    seed: &[u8; 32],
) -> Option<String> {
    let zone_key = members_zone_key(project, member_key, seed)?;
    let members = crate::members_file::read(root, &zone_key).ok()?;
    crate::privacy::email_for(project, member_key, Some(&members))
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
    member_key: &str,
    seed: &[u8; 32],
) -> usize {
    // By the at-rest KEY. The lookup was by address, and the caller has
    // handed it the member this device pinned since package J11 (D3.9),
    // which in an anonymous project is an opaque id that no address
    // matcher resolves: the member was not found, and a login that says
    // it re-locks quietly re-locked nothing.
    let Some(member) = project.member_by_key(member_key) else {
        return 0;
    };
    let mut relocked = 0;
    for (zone, wrap_hex) in &member.crypt_wraps {
        let Ok(zone_key) = joy_crypt::zone::unwrap_for_member(wrap_hex, zone, seed) else {
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
    zone_key: &joy_crypt::zone::ZoneKey,
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

fn relock_dir(dir: &Path, zone_key: &joy_crypt::zone::ZoneKey, zone: &str, relocked: &mut usize) {
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

fn relock_file(path: &Path, zone_key: &joy_crypt::zone::ZoneKey, zone: &str) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    if joy_crypt::zone::looks_like_blob(&bytes) {
        return false;
    }
    let blob = joy_crypt::zone::encrypt_blob(zone, zone_key, &bytes);
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
