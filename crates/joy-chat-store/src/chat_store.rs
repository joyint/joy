// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Where a sealed chat lives on `refs/joy/chats` (ADR JAPP-002A-30).
//!
//! Each chat is an opaque-id subtree of two content-addressed, grow-only
//! sets:
//!
//! ```text
//! <cid>/keys/<slot_id>   anonymous 108-byte content-key wraps
//! <cid>/log/<rid>        sealed events, one per fact
//! ```
//!
//! Nothing here opens or seals anything. This file reads those two sets
//! into a [`Sealed`] and writes back the bytes a save produced; the
//! opening, the epochs, the coverage and the delta live in
//! [`joy_chat::sealed`], which is why the app can run them in its webview
//! (JAPP-0135-FD). Merge stays a keyless union of both sets
//! ([`crate::chat_ref`]).
//!
//! Nothing about a chat is plaintext except the opaque tree names.

use std::collections::BTreeSet;

use git2::{FileMode, Repository, Tree};

use crate::chat_ref;
use joy_chat::chat_seal::{KEYS_DIR, LOG_DIR};
use joy_chat::chat_wrap::{ContentKey, SLOT_LEN};
use joy_chat::model::chat::Chat;
use joy_chat::sealed::{self, Sealed};
use joy_core::error::JoyError;

/// A chat-crypto failure in the store's own error language. The pure crate
/// knows nothing of joy-core, so the mapping lives on this side: an auth
/// failure stays one (a wrong seed, a slot for someone else), everything
/// else is a plain error.
fn from_chat(e: joy_chat::ChatError) -> JoyError {
    match e {
        joy_chat::ChatError::Auth(message) => JoyError::AuthFailed(message),
        other => JoyError::Other(other.to_string()),
    }
}
use joy_core::model::project::Project;

fn git(e: git2::Error) -> JoyError {
    JoyError::Git(e.to_string())
}

// ---- recipient resolution -------------------------------------------------

/// The member ids a chat's key must reach: its participants, or every
/// project member when the list is empty (the General/Team "everyone"
/// convention).
fn effective_recipient_ids(project: &Project, chat: &Chat) -> Vec<String> {
    use joy_chat::model::chat::ChatKind;
    if chat.participants.is_empty() && matches!(chat.kind, ChatKind::General | ChatKind::Team) {
        project.members().map(|(id, _)| id.clone()).collect()
    } else {
        chat.participants
            .iter()
            .map(|p| p.id().to_string())
            .collect()
    }
}

/// The verify_key on record for a recipient.
///
/// A recipient is a MEMBER, and nothing else. The reserved platform id
/// that used to sit next to them, so a server could read every chat, is
/// gone (JI-0174 family): a server reads a chat only where an AI member
/// it holds a token for is a participant, through that member's own key
/// like anyone else.
fn recipient_public(project: &Project, id: &str) -> Option<joy_core::auth::PublicKey> {
    let hex = project
        .member_by_key(id)
        .and_then(|m| m.verify_key.clone())?;
    joy_core::auth::PublicKey::from_hex(&hex).ok()
}

/// (recipient id, verify_key) for every current recipient, skipping
/// anyone without a verify_key on record.
fn recipients(project: &Project, chat: &Chat) -> Vec<(String, joy_core::auth::PublicKey)> {
    let mut out = Vec::new();
    for id in effective_recipient_ids(project, chat) {
        if let Some(pk) = recipient_public(project, &id) {
            out.push((id, pk));
        }
    }
    out
}

/// Whether this chat can be sealed here: a Joy project with at least one
/// wrap recipient, that is a participant with a verify_key. When false, the chat has no identity to encrypt for and
/// the caller keeps it plaintext / ephemeral per the ADR.
pub fn can_seal(root: &std::path::Path, chat: &Chat) -> bool {
    joy_core::store::load_project(root)
        .map(|p| !recipients(&p, chat).is_empty())
        .unwrap_or(false)
}

// ---- reading a stored chat ------------------------------------------------

/// The bytes of one chat plus the names already stored, so a save writes
/// only what is new.
struct Held {
    sealed: Sealed,
    slot_ids: BTreeSet<String>,
    log_rids: BTreeSet<String>,
}

fn subtree<'a>(repo: &'a Repository, parent: &Tree<'a>, name: &str) -> Option<Tree<'a>> {
    parent
        .get_name(name)
        .and_then(|e| e.to_object(repo).ok())
        .and_then(|o| o.peel_to_tree().ok())
}

/// A chat subtree is "new-format" iff it carries a `keys/` or `log/` set.
fn is_new_format(_repo: &Repository, chat_tree: &Tree) -> bool {
    chat_tree.get_name(KEYS_DIR).is_some() || chat_tree.get_name(LOG_DIR).is_some()
}

/// Read a chat subtree as bytes. No key is involved: this is the whole
/// point of the split.
fn read_subtree(repo: &Repository, chat_tree: &Tree) -> Held {
    let mut held = Held {
        sealed: Sealed::default(),
        slot_ids: BTreeSet::new(),
        log_rids: BTreeSet::new(),
    };
    if let Some(keys_tree) = subtree(repo, chat_tree, KEYS_DIR) {
        for e in keys_tree.iter() {
            let Some(name) = e.name() else { continue };
            let Ok(blob) = e.to_object(repo).and_then(|o| o.peel_to_blob()) else {
                continue;
            };
            if blob.content().len() == SLOT_LEN {
                held.sealed.slots.push(blob.content().to_vec());
                held.slot_ids.insert(name.to_string());
            }
        }
    }
    if let Some(log_tree) = subtree(repo, chat_tree, LOG_DIR) {
        for e in log_tree.iter() {
            let Some(name) = e.name() else { continue };
            let Ok(blob) = e.to_object(repo).and_then(|o| o.peel_to_blob()) else {
                continue;
            };
            held.log_rids.insert(name.to_string());
            held.sealed.blobs.push(blob.content().to_vec());
        }
    }
    held
}

/// What storage holds for one chat, as bytes. This is what a transport
/// hands to a client that owns the keys (JAPP-0135-FD): no seed is needed
/// to produce it, and it reveals nothing without one.
pub fn snapshot(root: &std::path::Path, cid: &str) -> Result<Option<Sealed>, JoyError> {
    let repo = chat_ref::open_repo(root)?;
    let Some(commit) = chat_ref::ref_commit(&repo)? else {
        return Ok(None);
    };
    let root_tree = commit.tree().map_err(git)?;
    let Some(chat_tree) = subtree(&repo, &root_tree, cid) else {
        return Ok(None);
    };
    if !is_new_format(&repo, &chat_tree) {
        return Ok(None);
    }
    Ok(Some(read_subtree(&repo, &chat_tree).sealed))
}

/// The same for every chat in the repository, by id.
pub fn snapshot_all(root: &std::path::Path) -> Result<Vec<(String, Sealed)>, JoyError> {
    let repo = chat_ref::open_repo(root)?;
    let Some(commit) = chat_ref::ref_commit(&repo)? else {
        return Ok(Vec::new());
    };
    let root_tree = commit.tree().map_err(git)?;
    let mut out = Vec::new();
    for e in root_tree.iter() {
        let Some(name) = e.name() else { continue };
        let Ok(chat_tree) = e.to_object(&repo).and_then(|o| o.peel_to_tree()) else {
            continue;
        };
        if !is_new_format(&repo, &chat_tree) {
            continue;
        }
        out.push((name.to_string(), read_subtree(&repo, &chat_tree).sealed));
    }
    Ok(out)
}

/// Persist `chat` sealed. `seed` is the writer's identity seed; it must
/// open the chat's existing keys to read the baseline and maintain
/// coverage. Idempotent: re-saving an unchanged chat writes no new
/// objects.
pub fn save(root: &std::path::Path, chat: &Chat, seed: &[u8; 32]) -> Result<(), JoyError> {
    // Read tip, build, move ref — and if another writer moved it first,
    // do the whole thing again on the new tip (JOY-023B-7E). Folding onto
    // the winner is the only way the loser's messages survive.
    for _ in 0..chat_ref::REF_MOVE_ATTEMPTS {
        if save_once(root, chat, seed)? {
            return Ok(());
        }
    }
    Err(JoyError::Other(
        "chat ref kept moving under this save; try again".into(),
    ))
}

/// One attempt of [`save`]; `false` means the ref moved underneath it.
fn save_once(root: &std::path::Path, chat: &Chat, seed: &[u8; 32]) -> Result<bool, JoyError> {
    let repo = chat_ref::open_repo(root)?;
    let project = joy_core::store::load_project(root)
        .map_err(|_| JoyError::AuthFailed("sealed chats need a Joy project".into()))?;
    let cid = chat.id.clone();

    // What is stored right now for this chat.
    let parent = chat_ref::ref_commit(&repo)?;
    let root_tree = match &parent {
        Some(c) => Some(c.tree().map_err(git)?),
        None => None,
    };
    let chat_tree = root_tree.as_ref().and_then(|t| subtree(&repo, t, &cid));
    // A legacy subtree is not read: the caller passes the already-opened
    // chat, the baseline starts empty, and the fresh <cid>/ tree carries
    // only keys/ + log/, so the old shape is dropped in the same commit.
    let held = match &chat_tree {
        Some(t) if is_new_format(&repo, t) => read_subtree(&repo, t),
        _ => Held {
            sealed: Sealed::default(),
            slot_ids: BTreeSet::new(),
            log_rids: BTreeSet::new(),
        },
    };
    // never inherit a legacy subtree's blobs when building the new tree.
    let base_tree = chat_tree.as_ref().filter(|t| is_new_format(&repo, t));

    // The two acts that need a key, both next door.
    let opened = sealed::open(&cid, &held.sealed, seed);
    let write =
        sealed::seal(&cid, &opened, chat, &recipients(&project, chat), seed).map_err(from_chat)?;

    write_tree(
        &repo,
        &cid,
        base_tree,
        &held.slot_ids,
        &held.log_rids,
        &write,
        parent.as_ref(),
        root_tree.as_ref(),
    )
}

#[allow(clippy::too_many_arguments)]
fn write_tree(
    repo: &Repository,
    cid: &str,
    chat_tree: Option<&Tree>,
    existing_slot_ids: &BTreeSet<String>,
    existing_log_rids: &BTreeSet<String>,
    write: &sealed::Write,
    parent: Option<&git2::Commit>,
    root_tree: Option<&Tree>,
) -> Result<bool, JoyError> {
    // keys/ subtree = existing + new slots (by content id, dedup).
    let mut keys_b = match chat_tree.and_then(|t| subtree(repo, t, KEYS_DIR)) {
        Some(t) => repo.treebuilder(Some(&t)).map_err(git)?,
        None => repo.treebuilder(None).map_err(git)?,
    };
    for (id, slot) in &write.slots {
        if !existing_slot_ids.contains(id) {
            let oid = repo.blob(slot).map_err(git)?;
            keys_b
                .insert(id, oid, i32::from(FileMode::Blob))
                .map_err(git)?;
        }
    }
    let keys_oid = keys_b.write().map_err(git)?;

    // log/ subtree = existing + newly sealed events (by rid, dedup).
    let mut log_b = match chat_tree.and_then(|t| subtree(repo, t, LOG_DIR)) {
        Some(t) => repo.treebuilder(Some(&t)).map_err(git)?,
        None => repo.treebuilder(None).map_err(git)?,
    };
    for (rid, blob) in &write.blobs {
        if !existing_log_rids.contains(rid) {
            let oid = repo.blob(blob).map_err(git)?;
            log_b
                .insert(rid, oid, i32::from(FileMode::Blob))
                .map_err(git)?;
        }
    }
    let log_oid = log_b.write().map_err(git)?;

    // <cid>/ subtree = { keys/, log/ }
    let mut chat_b = repo.treebuilder(None).map_err(git)?;
    chat_b
        .insert(KEYS_DIR, keys_oid, i32::from(FileMode::Tree))
        .map_err(git)?;
    chat_b
        .insert(LOG_DIR, log_oid, i32::from(FileMode::Tree))
        .map_err(git)?;
    let chat_oid = chat_b.write().map_err(git)?;

    // root tree with this chat spliced in.
    let mut root_b = match root_tree {
        Some(t) => repo.treebuilder(Some(t)).map_err(git)?,
        None => repo.treebuilder(None).map_err(git)?,
    };
    root_b
        .insert(cid, chat_oid, i32::from(FileMode::Tree))
        .map_err(git)?;
    let root_oid = root_b.write().map_err(git)?;
    let new_root = repo.find_tree(root_oid).map_err(git)?;
    let moved = chat_ref::commit_root(repo, parent, &new_root, &format!("chat {cid} [no-item]"))?;
    Ok(moved.is_some())
}

// ---- load -----------------------------------------------------------------

/// Load one new-format chat by id with a reader seed, folded. `None` if
/// absent or if the reader opens no key (not a participant). Legacy chats
/// are skipped here (read via the migration path).
pub fn load(root: &std::path::Path, id: &str, seed: &[u8; 32]) -> Result<Option<Chat>, JoyError> {
    let repo = chat_ref::open_repo(root)?;
    let Some(commit) = chat_ref::ref_commit(&repo)? else {
        return Ok(None);
    };
    let root_tree = commit.tree().map_err(git)?;
    let Some(chat_tree) = subtree(&repo, &root_tree, id) else {
        return Ok(None);
    };
    if !is_new_format(&repo, &chat_tree) {
        return Ok(None);
    }
    Ok(fold_subtree(&repo, id, &chat_tree, seed))
}

/// Every new-format chat the reader can open, folded (unsorted).
pub fn load_all(root: &std::path::Path, seed: &[u8; 32]) -> Result<Vec<Chat>, JoyError> {
    let repo = chat_ref::open_repo(root)?;
    let Some(commit) = chat_ref::ref_commit(&repo)? else {
        return Ok(Vec::new());
    };
    let root_tree = commit.tree().map_err(git)?;
    let mut out = Vec::new();
    for e in root_tree.iter() {
        let Some(name) = e.name() else { continue };
        let Ok(chat_tree) = e.to_object(&repo).and_then(|o| o.peel_to_tree()) else {
            continue;
        };
        if !is_new_format(&repo, &chat_tree) {
            continue;
        }
        if let Some(chat) = fold_subtree(&repo, name, &chat_tree, seed) {
            out.push(chat);
        }
    }
    Ok(out)
}

/// The content key of one chat epoch, resolved from the reader's seed
/// against the chat's key slots on `refs/joy/chats`. `None` if the chat is
/// absent or the reader holds no slot for that epoch (not a participant).
///
/// This is the key source `joy crypt` uses for a chat blob: the blob's own
/// zone header is `chat:<cid>#<epoch_id>`, so crypt follows that header to
/// the key here, exactly as a zone file follows its header to project.yaml.
pub fn epoch_content_key(
    root: &std::path::Path,
    cid: &str,
    epoch_id: &str,
    seed: &[u8; 32],
) -> Result<Option<ContentKey>, JoyError> {
    let repo = chat_ref::open_repo(root)?;
    let Some(commit) = chat_ref::ref_commit(&repo)? else {
        return Ok(None);
    };
    let root_tree = commit.tree().map_err(git)?;
    let Some(chat_tree) = subtree(&repo, &root_tree, cid) else {
        return Ok(None);
    };
    let held = read_subtree(&repo, &chat_tree);
    Ok(sealed::epoch_key(cid, &held.sealed, seed, epoch_id))
}

fn fold_subtree(repo: &Repository, id: &str, chat_tree: &Tree, seed: &[u8; 32]) -> Option<Chat> {
    let held = read_subtree(repo, chat_tree);
    let opened = sealed::open(id, &held.sealed, seed);
    if opened.epoch_keys.is_empty() {
        return None; // reader holds no key: not a participant
    }
    Some(opened.chat)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use joy_chat::model::chat::{ChatMessage, MessageKind};
    use joy_core::auth::IdentityKeypair;
    use joy_core::member_ref::MemberRef;
    use joy_core::model::project::{Member, MemberCapabilities};

    fn ts(s: u32) -> DateTime<Utc> {
        format!("2026-07-19T00:00:{s:02}Z").parse().unwrap()
    }
    fn msg(id: &str, s: u32, author: &str, text: &str) -> ChatMessage {
        ChatMessage {
            id: id.into(),
            at: ts(s),
            author: MemberRef::new(author),
            text: text.into(),
            kind: MessageKind::Text,
            delegated_by: None,
            turn_ms: None,
            tool_steps: None,
            tool: None,
            payload: None,
            details: None,
        }
    }

    /// A project with two members that hold identity keys; returns
    /// (root, horst_seed, anna_seed). There is no third party: whoever
    /// writes a chat holds a slot in it, like every other reader.
    fn project() -> (tempfile::TempDir, [u8; 32], [u8; 32]) {
        let dir = tempfile::tempdir().unwrap();
        joy_core::init::init(joy_core::init::InitOptions {
            root: dir.path().to_path_buf(),
            name: Some("Sealed".into()),
            acronym: Some("SL".into()),
            user: Some("horst@example.com".into()),
            language: None,
        })
        .unwrap();
        let (horst_seed, anna_seed) = ([1u8; 32], [3u8; 32]);
        let mut project = joy_core::store::load_project(dir.path()).unwrap();
        project
            .member_by_key_mut("horst@example.com")
            .unwrap()
            .verify_key = Some(
            IdentityKeypair::from_seed(&horst_seed)
                .public_key()
                .to_hex(),
        );
        let mut anna = Member::new(MemberCapabilities::All);
        anna.verify_key = Some(IdentityKeypair::from_seed(&anna_seed).public_key().to_hex());
        project.register_member("anna@example.com", anna).unwrap();
        joy_core::store::write_yaml(
            &joy_core::store::joy_dir(dir.path()).join(joy_core::store::PROJECT_FILE),
            &project,
        )
        .unwrap();
        (dir, horst_seed, anna_seed)
    }

    #[test]
    fn a_participant_and_the_custodian_read_what_the_custodian_saved() {
        let (dir, horst, _anna) = project();
        let mut chat = Chat::new("aaaa0000aaaa0000aaaa0000aaaa0000", vec![], ts(0));
        chat.title = Some("Standup".into());
        chat.participants = vec![MemberRef::new("horst@example.com")];
        chat.messages
            .push(msg("m1", 1, "horst@example.com", "secret plan"));

        // the custodian (platform) seals the chat
        save(dir.path(), &chat, &horst).unwrap();

        // the participant reads it back with their OWN seed
        let got = load(dir.path(), &chat.id, &horst).unwrap().unwrap();
        assert_eq!(got.title.as_deref(), Some("Standup"));
        assert_eq!(got.messages.len(), 1);
        assert_eq!(got.messages[0].text, "secret plan");

        // the custodian reads it too
        let cust = load(dir.path(), &chat.id, &horst).unwrap().unwrap();
        assert_eq!(cust.title.as_deref(), Some("Standup"));

        // no plaintext in the packed tree bytes
        assert_no_plaintext(dir.path(), &["Standup", "secret plan", "horst@example.com"]);
    }

    #[test]
    fn a_non_member_reads_nothing() {
        let (dir, horst, anna) = project();
        let mut chat = Chat::new("bbbb1111bbbb1111bbbb1111bbbb1111", vec![], ts(0));
        chat.participants = vec![MemberRef::new("horst@example.com")]; // NOT anna
        chat.title = Some("Private".into());
        chat.messages
            .push(msg("m1", 1, "horst@example.com", "hush"));
        save(dir.path(), &chat, &horst).unwrap();

        // anna is a project member but NOT a participant: opens nothing
        assert!(load(dir.path(), &chat.id, &anna).unwrap().is_none());
    }

    #[test]
    fn re_saving_an_unchanged_chat_adds_no_objects() {
        let (dir, horst, _a) = project();
        let mut chat = Chat::new("cccc2222cccc2222cccc2222cccc2222", vec![], ts(0));
        chat.participants = vec![MemberRef::new("horst@example.com")];
        chat.messages.push(msg("m1", 1, "horst@example.com", "one"));
        save(dir.path(), &chat, &horst).unwrap();

        let repo = Repository::open(dir.path()).unwrap();
        let tip1 = repo.refname_to_id(chat_ref::CHATS_REF).unwrap();
        // reload (as the custodian would) and save the unchanged chat
        let reloaded = load(dir.path(), &chat.id, &horst).unwrap().unwrap();
        save(dir.path(), &reloaded, &horst).unwrap();
        let tree1 = repo.find_commit(tip1).unwrap().tree().unwrap().id();
        let tip2 = repo.refname_to_id(chat_ref::CHATS_REF).unwrap();
        let tree2 = repo.find_commit(tip2).unwrap().tree().unwrap().id();
        assert_eq!(tree1, tree2, "idempotent: identical tree");
    }

    #[test]
    fn adding_a_message_appends_and_a_late_member_joins() {
        let (dir, horst, anna) = project();
        let mut chat = Chat::new("dddd3333dddd3333dddd3333dddd3333", vec![], ts(0));
        chat.participants = vec![MemberRef::new("horst@example.com")];
        chat.messages.push(msg("m1", 1, "horst@example.com", "one"));
        save(dir.path(), &chat, &horst).unwrap();

        // add anna + a second message
        let mut next = load(dir.path(), &chat.id, &horst).unwrap().unwrap();
        next.participants.push(MemberRef::new("anna@example.com"));
        next.messages.push(msg("m2", 2, "anna@example.com", "two"));
        save(dir.path(), &next, &horst).unwrap();

        // both read the full chat now
        let h = load(dir.path(), &chat.id, &horst).unwrap().unwrap();
        assert_eq!(h.messages.len(), 2);
        let a = load(dir.path(), &chat.id, &anna).unwrap().unwrap();
        assert_eq!(a.messages.len(), 2);
        assert_eq!(a.participants.len(), 2);
    }

    #[test]
    fn removing_a_member_rotates_forward_and_revokes_future_reads() {
        let (dir, horst, anna) = project();
        let mut chat = Chat::new("eeee4444eeee4444eeee4444eeee4444", vec![], ts(0));
        chat.participants = vec![
            MemberRef::new("horst@example.com"),
            MemberRef::new("anna@example.com"),
        ];
        chat.messages
            .push(msg("m1", 1, "horst@example.com", "before"));
        save(dir.path(), &chat, &horst).unwrap();
        // anna reads the pre-removal message
        assert_eq!(
            load(dir.path(), &chat.id, &anna)
                .unwrap()
                .unwrap()
                .messages
                .len(),
            1
        );

        // remove anna, add a post-removal message
        let mut next = load(dir.path(), &chat.id, &horst).unwrap().unwrap();
        next.participants.retain(|p| p.id() != "anna@example.com");
        next.messages
            .push(msg("m2", 2, "horst@example.com", "after anna left"));
        save(dir.path(), &next, &horst).unwrap();

        // horst (remaining) reads BOTH messages
        let h = load(dir.path(), &chat.id, &horst).unwrap().unwrap();
        assert_eq!(h.messages.len(), 2);

        // anna still reads her pre-removal history, NOT the new message
        let a = load(dir.path(), &chat.id, &anna).unwrap().unwrap();
        assert!(a.messages.iter().any(|m| m.text == "before"));
        assert!(
            !a.messages.iter().any(|m| m.text == "after anna left"),
            "removed member must not read post-rotation messages"
        );
    }

    #[test]
    fn a_key_change_re_wraps_and_the_new_key_reads_again() {
        let (dir, horst_old, _anna) = project();
        let mut chat = Chat::new("ffff5555ffff5555ffff5555ffff5555", vec![], ts(0));
        chat.participants = vec![MemberRef::new("horst@example.com")];
        chat.messages.push(msg("m1", 1, "horst@example.com", "hi"));
        save(dir.path(), &chat, &horst_old).unwrap();
        assert!(load(dir.path(), &chat.id, &horst_old).unwrap().is_some());

        // horst rotates his identity key in project.yaml
        let horst_new = [77u8; 32];
        let mut project = joy_core::store::load_project(dir.path()).unwrap();
        project
            .member_by_key_mut("horst@example.com")
            .unwrap()
            .verify_key = Some(IdentityKeypair::from_seed(&horst_new).public_key().to_hex());
        joy_core::store::write_yaml(
            &joy_core::store::joy_dir(dir.path()).join(joy_core::store::PROJECT_FILE),
            &project,
        )
        .unwrap();

        // the new key opens NOTHING yet (no slot for it)
        assert!(load(dir.path(), &chat.id, &horst_new).unwrap().is_none());
        // a save by the holder of the OLD key detects the change and re-wraps
        let reloaded = load(dir.path(), &chat.id, &horst_old).unwrap().unwrap();
        save(dir.path(), &reloaded, &horst_old).unwrap();
        // now the NEW key reads the full history
        let got = load(dir.path(), &chat.id, &horst_new).unwrap().unwrap();
        assert_eq!(got.messages.len(), 1);
        assert_eq!(got.messages[0].text, "hi");
    }

    #[test]
    fn divergent_sealed_saves_merge_keylessly() {
        let (dir, horst, _anna) = project();
        let mut chat = Chat::new("99998888999988889999888899998888", vec![], ts(0));
        chat.participants = vec![MemberRef::new("horst@example.com")];
        chat.messages
            .push(msg("m1", 1, "horst@example.com", "base"));
        save(dir.path(), &chat, &horst).unwrap();
        let repo = Repository::open(dir.path()).unwrap();
        let base_tip = repo.refname_to_id(chat_ref::CHATS_REF).unwrap();

        // lane A: add m2
        let mut a = load(dir.path(), &chat.id, &horst).unwrap().unwrap();
        a.messages.push(msg("m2", 2, "horst@example.com", "from A"));
        save(dir.path(), &a, &horst).unwrap();
        let a_tip = repo.refname_to_id(chat_ref::CHATS_REF).unwrap();

        // reset to base and diverge on lane B: add m3
        repo.reference(chat_ref::CHATS_REF, base_tip, true, "reset")
            .unwrap();
        let mut b = load(dir.path(), &chat.id, &horst).unwrap().unwrap();
        b.messages.push(msg("m3", 3, "horst@example.com", "from B"));
        save(dir.path(), &b, &horst).unwrap();
        let b_tip = repo.refname_to_id(chat_ref::CHATS_REF).unwrap();

        // keyless union of the two lanes
        chat_ref::merge_refs(dir.path(), a_tip, b_tip).unwrap();
        let merged = load(dir.path(), &chat.id, &horst).unwrap().unwrap();
        let texts: Vec<&str> = merged.messages.iter().map(|m| m.text.as_str()).collect();
        assert!(texts.contains(&"base"), "{texts:?}");
        assert!(texts.contains(&"from A"), "{texts:?}");
        assert!(texts.contains(&"from B"), "{texts:?}");
    }

    #[test]
    fn a_tool_result_enrichment_survives_the_second_save() {
        // append_tool_result / append_ai_reply persist TWICE: first the bare
        // message, then the enriched copy (tool+payload / attribution). The
        // event log must carry the enrichment — a diff that only emits NEW
        // message ids silently dropped it ("[tree result]" fallback bug).
        let (dir, horst, _anna) = project();
        let mut chat = Chat::new("5555cccc5555cccc5555cccc5555cccc", vec![], ts(0));
        chat.participants = vec![MemberRef::new("horst@example.com")];
        let mut m = msg("t1", 1, "horst@example.com", "[tree result]");
        m.kind = MessageKind::Tool;
        chat.messages.push(m);
        save(dir.path(), &chat, &horst).unwrap();

        // the enrichment save: same id, now with the frozen snapshot
        let mut next = load(dir.path(), &chat.id, &horst).unwrap().unwrap();
        next.messages[0].tool = Some("/joy".into());
        next.messages[0].payload = Some("{\"v\":1,\"result\":{\"kind\":\"tree\"}}".into());
        save(dir.path(), &next, &horst).unwrap();

        let got = load(dir.path(), &chat.id, &horst).unwrap().unwrap();
        assert_eq!(got.messages.len(), 1);
        assert_eq!(got.messages[0].tool.as_deref(), Some("/joy"));
        assert_eq!(
            got.messages[0].payload.as_deref(),
            Some("{\"v\":1,\"result\":{\"kind\":\"tree\"}}"),
            "the enriched payload must survive the second save"
        );
    }

    #[test]
    fn a_sealed_read_marker_round_trips() {
        let (dir, horst, _anna) = project();
        let mut chat = Chat::new("6666bbbb6666bbbb6666bbbb6666bbbb", vec![], ts(0));
        chat.participants = vec![MemberRef::new("horst@example.com")];
        chat.messages.push(msg("m1", 1, "horst@example.com", "hi"));
        save(dir.path(), &chat, &horst).unwrap();

        // horst reads it back and advances his read marker, then re-saves
        let mut h = load(dir.path(), &chat.id, &horst).unwrap().unwrap();
        h.read_markers.insert("horst@example.com".into(), ts(9));
        save(dir.path(), &h, &horst).unwrap();

        // the custodian reloads and sees horst's sealed watermark; nothing
        // about the marker is in plaintext
        let reloaded = load(dir.path(), &chat.id, &horst).unwrap().unwrap();
        assert_eq!(
            reloaded
                .read_markers
                .get("horst@example.com")
                .map(|d| d.timestamp()),
            Some(ts(9).timestamp())
        );
        assert_no_plaintext(dir.path(), &["horst@example.com"]);
    }

    #[test]
    fn a_chat_event_blob_decrypts_with_the_key_from_the_ref() {
        // Proves the `joy crypt` key source: a chat log blob is a standard
        // Crypt blob whose zone header is `chat:<cid>#<epoch>`; a participant
        // resolves the key from refs/joy/chats (not project.yaml) and
        // decrypt_blob yields the raw event YAML. A non-participant gets no
        // key. This is exactly what crypt's unlock_for_file does.
        let (dir, horst, anna) = project();
        let mut chat = Chat::new("7777aaaa7777aaaa7777aaaa7777aaaa", vec![], ts(0));
        chat.title = Some("Ops".into());
        chat.participants = vec![MemberRef::new("horst@example.com")]; // NOT anna
        chat.messages
            .push(msg("m1", 1, "horst@example.com", "top secret"));
        save(dir.path(), &chat, &horst).unwrap();

        let repo = Repository::open(dir.path()).unwrap();
        let commit = repo
            .find_commit(repo.refname_to_id(chat_ref::CHATS_REF).unwrap())
            .unwrap();
        let chat_tree = subtree(&repo, &commit.tree().unwrap(), &chat.id).unwrap();
        let log_tree = subtree(&repo, &chat_tree, LOG_DIR).unwrap();
        // any sealed blob names the chat+epoch in its Crypt header
        let sample = log_tree
            .iter()
            .next()
            .unwrap()
            .to_object(&repo)
            .unwrap()
            .peel_to_blob()
            .unwrap()
            .content()
            .to_vec();
        assert!(joy_crypt::zone::looks_like_blob(&sample));
        let zone_len = sample[9] as usize;
        let zone = std::str::from_utf8(&sample[10..10 + zone_len]).unwrap();
        let (cid, epoch) = zone
            .strip_prefix("chat:")
            .unwrap()
            .rsplit_once('#')
            .unwrap();
        assert_eq!(cid, chat.id);

        // a participant resolves the key FROM THE REF and reads raw YAML
        let ck = epoch_content_key(dir.path(), cid, epoch, &horst)
            .unwrap()
            .unwrap();
        let mut all = String::new();
        for e in log_tree.iter() {
            let b = e
                .to_object(&repo)
                .unwrap()
                .peel_to_blob()
                .unwrap()
                .content()
                .to_vec();
            let (_z, pt) = joy_crypt::zone::decrypt_blob(
                |_| Some(joy_crypt::zone::ZoneKey::from_bytes(ck)),
                &b,
            )
            .unwrap();
            all.push_str(&String::from_utf8(pt).unwrap());
        }
        assert!(
            all.contains("top secret"),
            "raw event YAML via ref key: {all}"
        );

        // a non-participant holds no slot: no key from the ref
        assert!(epoch_content_key(dir.path(), cid, epoch, &anna)
            .unwrap()
            .is_none());
    }

    fn assert_no_plaintext(root: &std::path::Path, needles: &[&str]) {
        // walk every blob reachable from the chats ref and assert none
        // contains a needle.
        let repo = Repository::open(root).unwrap();
        let commit = repo
            .find_commit(repo.refname_to_id(chat_ref::CHATS_REF).unwrap())
            .unwrap();
        let mut stack = vec![commit.tree().unwrap()];
        while let Some(tree) = stack.pop() {
            for e in tree.iter() {
                let obj = e.to_object(&repo).unwrap();
                if let Some(t) = obj.as_tree() {
                    stack.push(t.clone());
                } else if let Some(b) = obj.as_blob() {
                    let hay = b.content();
                    for n in needles {
                        assert!(
                            !contains(hay, n.as_bytes()),
                            "plaintext {n:?} leaked into a chat blob"
                        );
                    }
                }
            }
        }
        // also the commit message + author
        for n in needles {
            assert!(!commit.message().unwrap_or("").contains(n));
        }
    }

    fn contains(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }
}
