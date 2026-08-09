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

use crate::error::JoyError;
use crate::member_id::{email_match, opaque_member_id};
use crate::members_file::{self, MemberInfo, MembersFile, MEMBERS_ZONE};
use crate::model::project::{Member, PrivacyMode};
use crate::model::Project;
use crate::store;
use joy_crypt::zone::{unwrap_for_member, wrap_for_member, ZoneKey};

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
    project.member_key_for_email(email)
}

/// [`member_key_for_email`] plus the forge fallback (JOY-0253-8A, epic
/// JOY-0251-AA): when the address resolves no member, the project's
/// responsible forge plugin is asked whether it means anything (a forge
/// alias address, JP-00BF-94); the plugin's candidate addresses are then
/// tried through the SAME resolution. joy-core holds no forge knowledge:
/// which plugin is responsible is a `claims` question over the remotes
/// (project.yaml `forge:` stays the operator override), and whether the
/// address is an alias is the plugin's judgement alone.
///
/// A project without remotes, without installed plugins, or whose
/// remotes nobody claims behaves EXACTLY like [`member_key_for_email`].
/// Successful resolutions are cached per (root, email) for the process;
/// misses are not, so adding the member later works without a restart.
pub fn member_key_for_email_or_forge(
    project: &Project,
    root: &std::path::Path,
    email: &str,
    facts: Option<&crate::forge_plugins::CallerFacts>,
) -> Option<String> {
    if let Some(key) = member_key_for_email(project, email) {
        return Some(key);
    }
    if email.trim().is_empty() {
        return None;
    }
    if let Some(hit) = forge_cache_get(root, email) {
        // still a member? (a removed member must not resurrect via cache)
        if project.member_by_key(&hit).is_some() {
            return Some(hit);
        }
    }
    let remotes = crate::vcs::default_vcs()
        .all_remotes(root)
        .unwrap_or_default();
    let spec = crate::forge_plugins::responsible_plugin(project.forge.as_deref(), root, &remotes)?;
    let default_facts = crate::forge_plugins::CallerFacts::default();
    let facts = facts.unwrap_or(&default_facts);
    let acting = crate::forge_plugins::identity(spec, root, facts)?;
    // Direction one: the unresolved address belongs to the ACTOR (their
    // alias in the git config) and the plugin can vouch for their real
    // addresses — try those through the same resolution.
    if let Some(key) = member_key_for_any(project, &acting.emails) {
        forge_cache_put(root, email, &key);
        return Some(key);
    }
    // Direction two: the PROJECT is keyed by an alias (created under
    // one). `resolve` is pure by contract — it attributes an address
    // from the address alone — so a member key the plugin attributes to
    // the acting forge account IS the actor's slot. joy-core only
    // compares what two answers say; whether anything is an alias stays
    // the plugin's judgement.
    let key = project
        .members()
        .map(|(key, _)| key)
        .filter(|key| !key.starts_with("ai:"))
        .find(|key| {
            crate::forge_plugins::resolve(spec, root, key).is_some_and(|owner| {
                match (&owner.user_id, &acting.user_id) {
                    // the numeric account id is the strongest join
                    (Some(a), Some(b)) => a == b,
                    _ => owner.login.is_some() && owner.login == acting.login,
                }
            })
        })
        .cloned()?;
    forge_cache_put(root, email, &key);
    Some(key)
}

/// The first candidate address that IS a member, through the one
/// resolution. THE shared rule of the identity model (docs "Joy
/// Identität"): hosts differ only in where the candidates come from —
/// locally the forge plugin vouches for them, on the platform the
/// account's verified address set does.
pub fn member_key_for_any(project: &Project, candidates: &[String]) -> Option<String> {
    candidates
        .iter()
        .find_map(|candidate| member_key_for_email(project, candidate))
}

/// [`member_key_for_any`] plus the forge fallback (JOY-0253-8A): for a
/// multi-address caller (a platform account). Tries every address
/// directly, then consults the responsible forge plugin with the first
/// address as the unresolved one.
pub fn member_key_for_addresses_or_forge(
    project: &Project,
    root: &std::path::Path,
    addresses: &[String],
    facts: Option<&crate::forge_plugins::CallerFacts>,
) -> Option<String> {
    if let Some(key) = member_key_for_any(project, addresses) {
        return Some(key);
    }
    let first = addresses.first()?;
    member_key_for_email_or_forge(project, root, first, facts)
}

/// Positive forge resolutions per process: (root, unresolved email) ->
/// member key. Positives only — a miss re-runs, so config fixes and
/// late member adds take effect without restarting a long-lived host.
fn forge_cache(
) -> &'static std::sync::Mutex<std::collections::HashMap<(std::path::PathBuf, String), String>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<(std::path::PathBuf, String), String>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn forge_cache_get(root: &std::path::Path, email: &str) -> Option<String> {
    forge_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(root.to_path_buf(), email.to_string()))
        .cloned()
}

fn forge_cache_put(root: &std::path::Path, email: &str, key: &str) {
    forge_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert((root.to_path_buf(), email.to_string()), key.to_string());
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
        return project.has_member_key(member).then(|| member.to_string());
    }
    members.and_then(|m| m.email_for(member).map(str::to_string))
}

/// The at-rest representation of a delegating operator for the audit trail.
///
/// `operator_email` is the cleartext e-mail recorded in a delegation token
/// (create_delegation_token stores `operator_email` as the token's `delegated_by`
/// for the human-readable git trailer). When an AI acts under that delegation,
/// the operator is recorded as the `delegated-by:` part of the actor in items
/// (`created_by`/`updated_by`), logs, and the commit trailer. In `open` mode the
/// member key *is* the e-mail, returned as-is. In `anonymous` mode it resolves to
/// the operator's opaque member id, so no cleartext e-mail is written into a
/// committed file; `MemberRef` resolves it back for authorized display. Returns
/// `None` in anonymous mode when the operator is not a resolvable member, so a
/// cleartext e-mail is never written even as a fallback (ADR-042).
pub fn delegated_by_at_rest(project: &Project, operator_email: &str) -> Option<String> {
    match project.member_key_for_email(operator_email) {
        Some(key) => Some(key),
        None if project.privacy_mode() == PrivacyMode::Anonymous => None,
        None => Some(operator_email.to_string()),
    }
}

fn io_err(ctx: &str, e: std::io::Error) -> JoyError {
    JoyError::Other(format!("{ctx}: {e}"))
}

/// GDPR Art. 17 erasure: remove a member's e-mail and name from the encrypted
/// `members.yaml` and re-encrypt, severing the id -> PII resolution. The opaque
/// id, the `email_match` verifier and the whole audit trail in the versioned
/// files are deliberately left intact (Art. 17(3): the audit trail rests on a
/// legitimate interest). After this, no Joy output can resolve that id to a
/// person. Anonymous mode only; needs an operator seed with members.yaml access.
/// Returns whether an entry was actually removed.
pub fn erase_member(
    root: &Path,
    project: &Project,
    operator_seed: &[u8; 32],
    target_id: &str,
) -> Result<bool, JoyError> {
    if project.privacy_mode() != PrivacyMode::Anonymous {
        return Err(JoyError::Other(
            "erasure applies only to anonymous projects".into(),
        ));
    }
    let operator_vk = Keypair::from_seed(operator_seed).public_key().to_hex();
    let wrap = project
        .member_values()
        .find(|m| m.verify_key.as_deref() == Some(operator_vk.as_str()))
        .and_then(|m| m.members_wrap.clone())
        .ok_or_else(|| JoyError::Other("operator has no members.yaml access wrap".into()))?;
    let zone_key = unwrap_for_member(&wrap, MEMBERS_ZONE, operator_seed)?;
    let mut mf = members_file::read(root, &zone_key)?;
    let removed = mf.members.remove(target_id).is_some();
    if removed {
        members_file::write(root, &zone_key, &mf)?;
    }
    Ok(removed)
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

    for (key, mut member) in project.take_members() {
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
        let wrap = wrap_for_member(
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

    project.replace_members(new_members);
    project.set_privacy_mode(Some(PrivacyMode::Anonymous));

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
        .member_values()
        .find(|m| m.verify_key.as_deref() == Some(operator_vk.as_str()))
        .and_then(|m| m.members_wrap.clone())
        .ok_or_else(|| JoyError::Other("operator has no members.yaml access wrap".into()))?;
    let zone_key = unwrap_for_member(&wrap, MEMBERS_ZONE, operator_seed)?;
    let mf = members_file::read(root, &zone_key)?;

    let mut renamed: Vec<(String, String)> = Vec::new();
    let mut new_members: BTreeMap<String, Member> = BTreeMap::new();

    for (key, mut member) in project.take_members() {
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

    project.replace_members(new_members);
    project.set_privacy_mode(None);

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
    use joy_crypt::zone::looks_like_blob;

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
        project.register_member(EMAIL, member).unwrap();
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
        assert!(looks_like_blob(&raw));
        // project.yaml now keyed by opaque id, carries email_match.
        let pj: Project =
            store::read_yaml(&store::joy_dir(root).join(store::PROJECT_FILE)).unwrap();
        assert!(pj.has_member_key(&id));
        assert_eq!(pj.privacy_mode(), PrivacyMode::Anonymous);
        assert!(pj.member_by_key(&id).unwrap().email_match.is_some());

        // Switch back: e-mails restored, members.yaml gone.
        switch_to_open(root, &mut project, &seed).unwrap();
        assert!(!no_email_anywhere(root), "e-mail must be restored");
        assert!(!members_file::exists(root));
        let pj2: Project =
            store::read_yaml(&store::joy_dir(root).join(store::PROJECT_FILE)).unwrap();
        assert!(pj2.has_member_key(EMAIL));
        assert_eq!(pj2.privacy_mode(), PrivacyMode::Open);
        assert!(pj2.member_by_key(EMAIL).unwrap().email_match.is_none());
    }
}
