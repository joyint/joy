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
use joy_chat::chat_seal::{ATT_DIR, KEYS_DIR, LOG_DIR};
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

/// The project's members that can hold a key, in the shape
/// [`joy_chat::sealed`] wants. Who among them a given chat is for is the
/// pure crate's rule ([`sealed::recipients`]), not this file's.
fn sealing_members(project: &Project) -> Vec<sealed::Member> {
    project
        .members()
        .filter_map(|(id, m)| {
            let hex = m.verify_key.as_ref()?;
            let key = joy_core::auth::PublicKey::from_hex(hex).ok()?;
            Some(sealed::Member {
                id: id.clone(),
                verify_key: key,
            })
        })
        .collect()
}

/// (recipient id, verify_key) for every current recipient of this chat.
fn recipients(project: &Project, chat: &Chat) -> Vec<(String, joy_core::auth::PublicKey)> {
    sealed::recipients(chat, &sealing_members(project))
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
            let Ok(name) = e.name() else { continue };
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
            let Ok(name) = e.name() else { continue };
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
        let Ok(name) = e.name() else { continue };
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

/// Store the bytes a client produced for one chat, without a key.
///
/// This is the write half of the transport (JAPP-0135-FD): the webview
/// opened the chat, sealed the change and handed back named bytes; here
/// they are unioned into the tree and the ref moves. Whether the bytes
/// make sense is not decidable here, and does not have to be: a blob
/// nobody can open is a blob nobody can open, and the log is a
/// content-addressed set either way.
/// One sealed attachment blob of a chat, by its content-addressed name
/// (JOY-024C-97). The bytes are SEALED; opening them takes the reader's
/// epoch keys ([`joy_chat::sealed::open_attachment`]). `None` when the
/// chat or the attachment does not exist here.
pub fn attachment(
    root: &std::path::Path,
    cid: &str,
    name: &str,
) -> Result<Option<Vec<u8>>, JoyError> {
    let repo = chat_ref::open_repo(root)?;
    let Some(commit) = chat_ref::ref_commit(&repo)? else {
        return Ok(None);
    };
    let root_tree = commit.tree().map_err(git)?;
    let Some(chat_tree) = subtree(&repo, &root_tree, cid) else {
        return Ok(None);
    };
    let Some(att_tree) = subtree(&repo, &chat_tree, ATT_DIR) else {
        return Ok(None);
    };
    let Some(entry) = att_tree.get_name(name) else {
        return Ok(None);
    };
    let blob = repo.find_blob(entry.id()).map_err(git)?;
    Ok(Some(blob.content().to_vec()))
}

/// Store sealed attachment blobs for a chat (JOY-024C-97): the write half
/// for a host that sealed them itself ([`joy_chat::sealed::seal_attachment`]).
pub fn put_attachments(
    root: &std::path::Path,
    cid: &str,
    attachments: Vec<(String, Vec<u8>)>,
) -> Result<(), JoyError> {
    commit(
        root,
        cid,
        &sealed::Write {
            attachments,
            ..sealed::Write::default()
        },
    )
}

pub fn commit(root: &std::path::Path, cid: &str, write: &sealed::Write) -> Result<(), JoyError> {
    if write.is_empty() {
        return Ok(());
    }
    for _ in 0..chat_ref::REF_MOVE_ATTEMPTS {
        if commit_once(root, cid, write)? {
            return Ok(());
        }
    }
    Err(JoyError::Other(
        "chat ref kept moving under this write; try again".into(),
    ))
}

fn commit_once(root: &std::path::Path, cid: &str, write: &sealed::Write) -> Result<bool, JoyError> {
    let repo = chat_ref::open_repo(root)?;
    let parent = chat_ref::ref_commit(&repo)?;
    let root_tree = match &parent {
        Some(c) => Some(c.tree().map_err(git)?),
        None => None,
    };
    let chat_tree = root_tree.as_ref().and_then(|t| subtree(&repo, t, cid));
    let held = match &chat_tree {
        Some(t) if is_new_format(&repo, t) => read_subtree(&repo, t),
        _ => Held {
            sealed: Sealed::default(),
            slot_ids: BTreeSet::new(),
            log_rids: BTreeSet::new(),
        },
    };
    let base_tree = chat_tree.as_ref().filter(|t| is_new_format(&repo, t));
    write_tree(
        &repo,
        cid,
        base_tree,
        &held.slot_ids,
        &held.log_rids,
        write,
        parent.as_ref(),
        root_tree.as_ref(),
    )
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

    // att/ subtree = existing + newly sealed attachments (content
    // addressed, so a re-insert of the same name is the same blob).
    let mut att_b = match chat_tree.and_then(|t| subtree(repo, t, ATT_DIR)) {
        Some(t) => repo.treebuilder(Some(&t)).map_err(git)?,
        None => repo.treebuilder(None).map_err(git)?,
    };
    for (aid, blob) in &write.attachments {
        let oid = repo.blob(blob).map_err(git)?;
        att_b
            .insert(aid, oid, i32::from(FileMode::Blob))
            .map_err(git)?;
    }
    let att_len = att_b.len();
    let att_oid = att_b.write().map_err(git)?;

    // <cid>/ subtree = { keys/, log/ [, att/] }
    let mut chat_b = repo.treebuilder(None).map_err(git)?;
    chat_b
        .insert(KEYS_DIR, keys_oid, i32::from(FileMode::Tree))
        .map_err(git)?;
    chat_b
        .insert(LOG_DIR, log_oid, i32::from(FileMode::Tree))
        .map_err(git)?;
    if att_len > 0 {
        chat_b
            .insert(ATT_DIR, att_oid, i32::from(FileMode::Tree))
            .map_err(git)?;
    }
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
        let Ok(name) = e.name() else { continue };
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
            parts: Vec::new(),
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

    /// Demo reproduction (JP-00E4-B3): horst creates a chat and its FIRST
    /// message mentions the AI. The chat must stay readable to horst, its
    /// creator. If it ends up sealed only to the AI, horst loses it: it
    /// hides, rename fails to decrypt, and the AI never answers.
    #[test]
    fn a_chat_whose_first_message_mentions_an_ai_stays_readable_to_its_creator() {
        let (dir, horst, _anna) = project();
        // add an AI member with its own key (like ai:vibe@joy)
        let ai_seed = [9u8; 32];
        let mut project = joy_core::store::load_project(dir.path()).unwrap();
        let mut vibe = joy_core::model::project::Member::new(
            joy_core::model::project::MemberCapabilities::All,
        );
        vibe.verify_key = Some(IdentityKeypair::from_seed(&ai_seed).public_key().to_hex());
        project.register_member("ai:vibe@joy", vibe).unwrap();
        joy_core::store::write_yaml(
            &joy_core::store::joy_dir(dir.path()).join(joy_core::store::PROJECT_FILE),
            &project,
        )
        .unwrap();

        // horst opens a fresh direct chat (participants = himself)
        let mut chat = Chat::new("cccc1111cccc1111cccc1111cccc1111", vec![], ts(0));
        chat.participants = vec![MemberRef::new("horst@example.com")];
        save(dir.path(), &chat, &horst).unwrap();
        assert!(
            load(dir.path(), &chat.id, &horst).unwrap().is_some(),
            "own chat readable at create"
        );

        // his first line mentions the AI; the app adds the AI as a
        // participant (chat_turns::add_mentioned_ais), then seals. horst
        // is the writer for both.
        crate::writer::set_thread_seed(Some(Some(horst)));
        let mut reloaded = load(dir.path(), &chat.id, &horst).unwrap().unwrap();
        crate::chats::append_message(
            dir.path(),
            &mut reloaded,
            MemberRef::new("horst@example.com"),
            "@vibe please help",
            ts(1),
        )
        .unwrap();
        crate::chats::add_participant(
            dir.path(),
            &mut reloaded,
            MemberRef::new("ai:vibe@joy"),
            &MemberRef::new("horst@example.com"),
            ts(2),
        )
        .unwrap();

        // horst must STILL read his own chat
        let got = load(dir.path(), &chat.id, &horst).unwrap();
        assert!(
            got.is_some(),
            "creator lost read access after the AI joined"
        );
        // and the AI can read it too
        assert!(
            load(dir.path(), &chat.id, &ai_seed).unwrap().is_some(),
            "AI cannot read"
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

#[cfg(test)]
mod attachment_tests {
    use super::*;
    use joy_chat::model::chat::{Chat, MessagePart};

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        dir
    }

    /// JOY-024C-97 end to end on the store: seal an attachment, store it
    /// next to the message, read it back sealed, open it with the
    /// reader's keys — and a foreign seed opens nothing.
    #[test]
    fn an_attachment_seals_stores_and_opens_only_for_a_recipient() {
        let dir = repo();
        let root = dir.path();
        let seed = [7u8; 32];
        let vk = joy_core::auth::IdentityKeypair::from_seed(&seed)
            .public_key()
            .to_hex();
        let members = [sealed::Member {
            id: "alice@example.com".into(),
            verify_key: joy_core::auth::PublicKey::from_hex(&vk).unwrap(),
        }];

        // a chat with one message whose part references the attachment
        let mut chat = Chat::new(
            "c-att",
            vec![joy_model::MemberRef::new("alice@example.com")],
            chrono::Utc::now(),
        );
        let opened = sealed::open("c-att", &Sealed::default(), &seed);
        let payload = b"png bytes stand in";
        // no epoch yet: seal the chat first to mint one, then the blob
        let recipients = sealed::recipients(&chat, &members);
        let write = sealed::seal("c-att", &opened, &chat, &recipients, &seed).unwrap();
        commit(root, "c-att", &write).unwrap();
        let sealed_now = snapshot(root, "c-att").unwrap().unwrap();
        let opened = sealed::open("c-att", &sealed_now, &seed);
        let (aid, blob) = sealed::seal_attachment("c-att", &opened, payload).unwrap();
        put_attachments(root, "c-att", vec![(aid.clone(), blob)]).unwrap();

        chat.messages.push(joy_chat::model::chat::ChatMessage {
            id: "m1".into(),
            at: chrono::Utc::now(),
            author: joy_model::MemberRef::new("alice@example.com"),
            text: "see the screenshot".into(),
            kind: Default::default(),
            delegated_by: None,
            turn_ms: None,
            tool_steps: None,
            tool: None,
            payload: None,
            details: None,
            parts: vec![MessagePart::Image {
                mime: "image/png".into(),
                attachment: aid.clone(),
                label: "screenshot".into(),
            }],
        });
        let write = sealed::seal("c-att", &opened, &chat, &recipients, &seed).unwrap();
        commit(root, "c-att", &write).unwrap();

        // the message and its part survive the round trip
        let loaded = load(root, "c-att", &seed).unwrap().unwrap();
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.messages[0].parts.len(), 1);
        assert_eq!(loaded.messages[0].parts[0].attachment(), Some(aid.as_str()));

        // the stored blob is sealed; the recipient opens it, a stranger not
        let stored = attachment(root, "c-att", &aid).unwrap().unwrap();
        assert_ne!(stored.as_slice(), payload);
        let sealed_now = snapshot(root, "c-att").unwrap().unwrap();
        let opened = sealed::open("c-att", &sealed_now, &seed);
        assert_eq!(
            sealed::open_attachment(&opened, &stored).as_deref(),
            Some(payload.as_slice())
        );
        let stranger = sealed::open("c-att", &sealed_now, &[9u8; 32]);
        assert!(sealed::open_attachment(&stranger, &stored).is_none());

        // an unknown name is None, not an error
        assert!(attachment(root, "c-att", "feedbeef").unwrap().is_none());
    }

    /// The cap is an honest refusal, not a truncation.
    #[test]
    fn an_oversized_attachment_is_refused() {
        let dir = repo();
        let root = dir.path();
        let _ = root;
        let opened = {
            // an opened chat with one held epoch key
            let mut epoch_keys = std::collections::BTreeMap::new();
            epoch_keys.insert("e1".to_string(), joy_chat::chat_wrap::new_content_key());
            sealed::Opened {
                chat: Chat::new("c", Vec::new(), chrono::Utc::now()),
                events: Vec::new(),
                epoch_keys,
            }
        };
        let big = vec![0u8; joy_chat::chat_seal::MAX_ATTACHMENT_BYTES + 1];
        let err = sealed::seal_attachment("c", &opened, &big).unwrap_err();
        assert!(err.to_string().contains("exceeds"), "{err}");
    }
}
