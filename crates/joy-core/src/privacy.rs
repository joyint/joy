// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Privacy-mode migration (ADR-042).
//!
//! Switches a project's *working* `.joy/` files between `open` (cleartext
//! e-mail) and `anonymous` (opaque ids + encrypted `members.yaml`). The switch
//! is one atomic, deliberate operation: it rekeys the member map, writes the
//! verifier and the encrypted members file, and rewrites every item and log so
//! no member e-mail remains in any working file. Switching back restores them.
//!
//! Git *commit history* is deliberately out of scope: old commits keep their
//! e-mails, which only a history rewrite could change. The guarantee here is
//! about the working tree.
//!
//! The migration requires the operator's unlocked identity seed (auth) and the
//! manage capability; both are enforced by the caller (`joy project set`).

use std::collections::BTreeMap;
use std::path::Path;

use joy_crypt::identity::{Keypair, PublicKey};

use crate::crypt::{self, ZoneKey};
use crate::error::JoyError;
use crate::member_id::{email_match, opaque_member_id};
use crate::members_file::{self, MemberInfo, MembersFile, MEMBERS_ZONE};
use crate::model::project::{Member, PrivacyMode};
use crate::model::Project;
use crate::store;

/// A human member is one whose map key is an e-mail (not an `ai:` synthetic id).
/// Only human members carry PII and get anonymized; AI members keep their
/// readable synthetic id.
fn is_human_key(key: &str) -> bool {
    !key.starts_with("ai:")
}

/// Resolve the member-map key for a git e-mail, honoring the privacy mode. In
/// `open` mode the key is the e-mail itself; in `anonymous` mode it is the
/// opaque id whose stored `email_match` verifies against the e-mail. Returns
/// `None` when the e-mail is not a member.
pub fn member_key_for_email(project: &Project, email: &str) -> Option<String> {
    if project.privacy_mode() != PrivacyMode::Anonymous {
        return project
            .members
            .contains_key(email)
            .then(|| email.to_string());
    }
    for (id, member) in &project.members {
        if let (Some(verifier), Some(nonce)) = (&member.email_match, &member.kdf_nonce) {
            if email_match(email, nonce).ok().as_deref() == Some(verifier.as_str()) {
                return Some(id.clone());
            }
        }
    }
    None
}

/// The single source of a member's e-mail (the concept's `email_for`).
///
/// Open mode: the member-map key *is* the e-mail, returned as-is. Anonymous
/// mode: the e-mail lives only in the decrypted `members.yaml`, looked up by
/// the opaque id. Every consumer that needs a member's e-mail (attestation
/// verification, display, account matching) goes through here and is otherwise
/// oblivious to the privacy mode. Returns `None` when the member is unknown or,
/// in anonymous mode, when `members` is not available (locked).
pub fn email_for(
    project: &Project,
    member: &str,
    members: Option<&crate::members_file::MembersFile>,
) -> Option<String> {
    if project.privacy_mode() != PrivacyMode::Anonymous {
        return project
            .members
            .contains_key(member)
            .then(|| member.to_string());
    }
    members.and_then(|m| m.email_for(member).map(str::to_string))
}

fn io_err(ctx: &str, e: std::io::Error) -> JoyError {
    JoyError::Other(format!("{ctx}: {e}"))
}

/// Replace each `from -> to` in a single text file, if present.
fn rewrite_file(path: &Path, replacements: &[(String, String)]) -> Result<(), JoyError> {
    if !path.exists() {
        return Ok(());
    }
    let mut content = std::fs::read_to_string(path).map_err(|e| io_err("read", e))?;
    let mut changed = false;
    for (from, to) in replacements {
        if !from.is_empty() && content.contains(from.as_str()) {
            content = content.replace(from.as_str(), to);
            changed = true;
        }
    }
    if changed {
        std::fs::write(path, content).map_err(|e| io_err("write", e))?;
    }
    Ok(())
}

/// Replace each `from -> to` across every `*.<ext>` file in `dir`.
fn rewrite_dir(dir: &Path, ext: &str, replacements: &[(String, String)]) -> Result<(), JoyError> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).map_err(|e| io_err("read_dir", e))? {
        let path = entry.map_err(|e| io_err("read_dir entry", e))?.path();
        if path.extension().and_then(|e| e.to_str()) == Some(ext) {
            rewrite_file(&path, replacements)?;
        }
    }
    Ok(())
}

/// Rewrite project.yaml, all items, and all logs with the given substitutions.
/// Used to scrub residual e-mails (attestation `attester` / `signed_fields`,
/// item `created_by` / `assignees` / comment authors, log actors) on switch in,
/// and to restore them on switch out.
fn rewrite_working_tree(root: &Path, replacements: &[(String, String)]) -> Result<(), JoyError> {
    let joy = store::joy_dir(root);
    rewrite_file(&joy.join(store::PROJECT_FILE), replacements)?;
    rewrite_dir(&joy.join(store::ITEMS_DIR), "yaml", replacements)?;
    rewrite_dir(&joy.join(store::LOG_DIR), "log", replacements)?;
    Ok(())
}

/// Remove a top-level key from project.yaml. Needed because
/// `write_yaml_preserve` keeps keys present in the original file but absent from
/// the serialized struct (so a `privacy` field cleared to `None` would otherwise
/// linger as the stale `anonymous` value).
fn prune_yaml_key(path: &Path, key: &str) -> Result<(), JoyError> {
    use serde_yaml_ng::Value;
    let raw = std::fs::read_to_string(path).map_err(|e| io_err("read", e))?;
    let mut value: Value = serde_yaml_ng::from_str(&raw)?;
    if let Some(map) = value.as_mapping_mut() {
        map.remove(Value::String(key.to_string()));
    }
    let yaml = serde_yaml_ng::to_string(&value)?;
    std::fs::write(path, yaml).map_err(|e| io_err("write", e))?;
    Ok(())
}

/// Switch a project from `open` to `anonymous`.
///
/// `operator_seed` is the unlocked identity seed of the manage member running
/// the switch; it grants every member access to the members.yaml zone key.
/// Returns the `(email, opaque_id)` pairs that were anonymized.
pub fn switch_to_anonymous(
    root: &Path,
    project: &mut Project,
    operator_seed: &[u8; 32],
) -> Result<Vec<(String, String)>, JoyError> {
    if project.privacy_mode() == PrivacyMode::Anonymous {
        return Err(JoyError::Other("project is already anonymous".into()));
    }

    let operator_pk = Keypair::from_seed(operator_seed).public_key();
    let zone_key = ZoneKey::generate();

    let mut renamed: Vec<(String, String)> = Vec::new();
    let mut new_members: BTreeMap<String, Member> = BTreeMap::new();
    let mut mf = MembersFile::default();

    for (key, mut member) in std::mem::take(&mut project.members) {
        if !is_human_key(&key) {
            // AI member: keep synthetic id and entry as-is.
            new_members.insert(key, member);
            continue;
        }
        let email = key;
        let verify_key = member.verify_key.clone().ok_or_else(|| {
            JoyError::Other(format!(
                "member {email} has no verify_key; run joy auth init first"
            ))
        })?;
        let kdf_nonce = member
            .kdf_nonce
            .clone()
            .ok_or_else(|| JoyError::Other(format!("member {email} has no kdf_nonce")))?;

        let id = opaque_member_id(&verify_key)
            .map_err(|e| JoyError::Other(format!("bad verify_key for {email}: {e}")))?;
        let verifier = email_match(&email, &kdf_nonce)
            .map_err(|e| JoyError::Other(format!("bad kdf_nonce for {email}: {e}")))?;

        let recipient_pk = PublicKey::from_hex(&verify_key)?;
        let wrap = crypt::wrap_for_member(
            &zone_key,
            MEMBERS_ZONE,
            operator_seed,
            &operator_pk,
            &recipient_pk,
        );

        member.email_match = Some(verifier);
        member.members_wrap = Some(wrap);

        mf.members.insert(
            id.clone(),
            MemberInfo {
                email: email.clone(),
                name: None,
            },
        );
        renamed.push((email, id.clone()));
        new_members.insert(id, member);
    }

    project.members = new_members;
    project.privacy = Some(PrivacyMode::Anonymous);

    // Persist the structural changes, then scrub residual e-mails (attestation
    // fields in project.yaml, item bodies, logs) by textual substitution.
    let project_path = store::joy_dir(root).join(store::PROJECT_FILE);
    store::write_yaml_preserve(&project_path, project)?;
    members_file::write(root, &zone_key, &mf)?;
    rewrite_working_tree(root, &renamed)?;

    Ok(renamed)
}

/// Switch a project from `anonymous` back to `open`.
///
/// `operator_seed` must belong to a member with a members.yaml wrap so the zone
/// key can be unwrapped and the e-mails recovered.
pub fn switch_to_open(
    root: &Path,
    project: &mut Project,
    operator_seed: &[u8; 32],
) -> Result<Vec<(String, String)>, JoyError> {
    if project.privacy_mode() != PrivacyMode::Anonymous {
        return Err(JoyError::Other("project is not anonymous".into()));
    }

    // Find the operator's own entry (its verify_key matches the seed) to get the
    // members.yaml wrap, then unwrap the zone key with the operator's seed.
    let operator_vk = Keypair::from_seed(operator_seed).public_key().to_hex();
    let wrap = project
        .members
        .values()
        .find(|m| m.verify_key.as_deref() == Some(operator_vk.as_str()))
        .and_then(|m| m.members_wrap.clone())
        .ok_or_else(|| JoyError::Other("operator has no members.yaml access wrap".into()))?;
    let zone_key = crypt::unwrap_for_member(&wrap, MEMBERS_ZONE, operator_seed)?;
    let mf = members_file::read(root, &zone_key)?;

    let mut renamed: Vec<(String, String)> = Vec::new();
    let mut new_members: BTreeMap<String, Member> = BTreeMap::new();

    for (key, mut member) in std::mem::take(&mut project.members) {
        if !is_human_key(&key) && !mf.members.contains_key(&key) {
            new_members.insert(key, member);
            continue;
        }
        match mf.email_for(&key) {
            Some(email) => {
                member.email_match = None;
                member.members_wrap = None;
                renamed.push((key.clone(), email.to_string()));
                new_members.insert(email.to_string(), member);
            }
            None => {
                // Not in members.yaml (e.g. an AI member): keep as-is.
                new_members.insert(key, member);
            }
        }
    }

    project.members = new_members;
    project.privacy = None;

    let project_path = store::joy_dir(root).join(store::PROJECT_FILE);
    store::write_yaml_preserve(&project_path, project)?;
    prune_yaml_key(&project_path, "privacy")?;
    // Remove the encrypted members file and restore e-mails in the working tree.
    let mp = members_file::members_path(root);
    if mp.exists() {
        std::fs::remove_file(&mp).map_err(|e| io_err("remove members.yaml", e))?;
    }
    rewrite_working_tree(root, &renamed)?;

    Ok(renamed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::project::MemberCapabilities;

    const EMAIL: &str = "test@example.com";
    const NONCE: &str = "8c1f00000000000000000000000000000000000000000000000000000000e4ab";

    fn setup(root: &Path, seed: &[u8; 32]) -> Project {
        let joy = store::joy_dir(root);
        std::fs::create_dir_all(joy.join(store::ITEMS_DIR)).unwrap();
        std::fs::create_dir_all(joy.join(store::LOG_DIR)).unwrap();

        let vk = Keypair::from_seed(seed).public_key().to_hex();
        let mut member = Member::new(MemberCapabilities::All);
        member.verify_key = Some(vk);
        member.kdf_nonce = Some(NONCE.to_string());

        let mut project = Project::new("Test".into(), Some("T".into()));
        project.members.insert(EMAIL.to_string(), member);
        store::write_yaml_preserve(&joy.join(store::PROJECT_FILE), &project).unwrap();

        // An item assigned to the human, and a log line naming them.
        std::fs::write(
            joy.join(store::ITEMS_DIR).join("T-0001-x.yaml"),
            format!("id: T-0001\ntitle: x\nassignees:\n- member: {EMAIL}\ncreated_by: {EMAIL}\n"),
        )
        .unwrap();
        std::fs::write(
            joy.join(store::LOG_DIR).join("2026-06-11.log"),
            format!("2026-06-11T09:00:00Z T-0001 item.created [{EMAIL}]\n"),
        )
        .unwrap();

        project
    }

    fn no_email_anywhere(root: &Path) -> bool {
        let joy = store::joy_dir(root);
        for sub in [store::PROJECT_FILE] {
            if std::fs::read_to_string(joy.join(sub))
                .unwrap()
                .contains(EMAIL)
            {
                return false;
            }
        }
        for dir in [store::ITEMS_DIR, store::LOG_DIR] {
            for entry in std::fs::read_dir(joy.join(dir)).unwrap() {
                let p = entry.unwrap().path();
                if std::fs::read(&p)
                    .unwrap()
                    .windows(EMAIL.len())
                    .any(|w| w == EMAIL.as_bytes())
                {
                    return false;
                }
            }
        }
        true
    }

    #[test]
    fn switch_round_trip_scrubs_then_restores_emails() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let seed = [7u8; 32];
        let mut project = setup(root, &seed);

        // Before: the e-mail is present.
        assert!(!no_email_anywhere(root));

        // Switch to anonymous: no e-mail anywhere, members.yaml is an encrypted blob.
        let renamed = switch_to_anonymous(root, &mut project, &seed).unwrap();
        assert_eq!(renamed.len(), 1);
        let id = renamed[0].1.clone();
        assert!(id.starts_with("m-"));
        assert!(
            no_email_anywhere(root),
            "no e-mail must remain after switch"
        );
        assert!(members_file::exists(root));
        let raw = std::fs::read(members_file::members_path(root)).unwrap();
        assert!(crypt::looks_like_blob(&raw));
        // project.yaml now keyed by opaque id, carries email_match.
        let pj: Project =
            store::read_yaml(&store::joy_dir(root).join(store::PROJECT_FILE)).unwrap();
        assert!(pj.members.contains_key(&id));
        assert_eq!(pj.privacy, Some(PrivacyMode::Anonymous));
        assert!(pj.members[&id].email_match.is_some());

        // Switch back: e-mails restored, members.yaml gone.
        switch_to_open(root, &mut project, &seed).unwrap();
        assert!(!no_email_anywhere(root), "e-mail must be restored");
        assert!(!members_file::exists(root));
        let pj2: Project =
            store::read_yaml(&store::joy_dir(root).join(store::PROJECT_FILE)).unwrap();
        assert!(pj2.members.contains_key(EMAIL));
        assert_eq!(pj2.privacy, None);
        assert!(pj2.members[EMAIL].email_match.is_none());
    }
}
