// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Chat storage on a dedicated git ref (ADR JAPP-00DC-FC).
//!
//! Chats live on `refs/joy/chats`, NOT on the working branch, so chat
//! activity never floods the development git log. The ref is a normal git
//! ref outside `refs/heads/`: it is never checked out, `git log` and a
//! plain `git pull` ignore it, yet it is versioned and pushed/fetched
//! explicitly by the sync layer (platform / desktop).
//!
//! Storage is append-only per message so a merge unions messages by id
//! (the prerequisite for future encryption): the tree is
//!
//! ```text
//! <chat-id>/meta.yaml               # the chat without its messages
//! <chat-id>/messages/<msg-id>.yaml  # one blob per message
//! ```
//!
//! `meta.yaml` is the [`Chat`] minus its messages: identity, title,
//! participants, `ai_sessions` (ACP session per AI member), and
//! `interaction-levels` — the per-delegator level overrides (ADR
//! JAPP-00F3-E8 as revised by JI-0166-D8), a nested map of AI
//! participant id to delegating member id to
//! `proposing | confirmed | autonomous`:
//!
//! ```text
//! ai_sessions:
//!   ai:claude@joy: acp-session-42
//! interaction-levels:
//!   ai:claude@joy:
//!     horst@example.com: confirmed
//! ```
//!
//! Nested YAML maps merge key by key ([`joy_core::merge`]), so concurrent
//! writes to DIFFERENT delegator entries union cleanly; a concurrent
//! write to the SAME entry resolves by the chat's `updated` timestamp.
//!
//! This is the single place that knows the on-disk-in-git shape of a
//! chat. [`crate::chats`] is the semantic layer on top (visibility,
//! delete rules, turn logic) and never touches git directly.

use std::collections::BTreeSet;
use std::path::Path;

use chrono::Utc;
use git2::{Commit, ErrorCode, FileMode, Oid, Repository, Signature, Time, Tree};

use crate::model::chat::{Chat, ChatMessage};
use joy_core::error::JoyError;

/// The dedicated ref chats live on. Outside `refs/heads/`, so it never
/// appears in `git log`, `git branch`, or a plain `git pull`.
pub const CHATS_REF: &str = "refs/joy/chats";

/// The local tracking ref a fetch of [`CHATS_REF`] lands on before the
/// reconcile (every sync path uses the same name, so a half-finished sync
/// never leaks state between callers).
pub const CHATS_TRACKING_REF: &str = "refs/joy/chats-remote";

const META_FILE: &str = "meta.yaml";
const MESSAGES_DIR: &str = "messages";

fn git(e: git2::Error) -> JoyError {
    JoyError::Git(e.to_string())
}

/// Open the repository containing `root` (walks up like git does).
pub(crate) fn open_repo(root: &Path) -> Result<Repository, JoyError> {
    Repository::discover(root).map_err(git)
}

/// The signature for EVERY chat-ref commit, on every device: a FIXED
/// neutral identity plus a day-coarsened time (ADR JAPP-002A-30). A chat
/// must leak nothing to a keyless repo reader, including WHO touched it:
/// `repo.signature()` stamped each commit with the writer's real
/// name/email, so `git log --stat refs/joy/chats` mapped a member to an
/// (opaque) chat regardless of how sealed the tree was. Chat ordering is
/// by the in-chat data (message `at`, `updated`), never by git
/// author/time, so a constant identity and coarse time lose nothing.
pub(crate) fn signature(_repo: &Repository) -> Result<Signature<'static>, JoyError> {
    let day = (Utc::now().timestamp() / 86_400) * 86_400;
    Signature::new("joy", "joy@localhost", &Time::new(day, 0)).map_err(git)
}

/// The current `refs/joy/chats` commit, or `None` if the ref is unborn.
pub(crate) fn ref_commit(repo: &Repository) -> Result<Option<Commit<'_>>, JoyError> {
    match repo.refname_to_id(CHATS_REF) {
        Ok(oid) => Ok(Some(repo.find_commit(oid).map_err(git)?)),
        Err(e) if e.code() == ErrorCode::NotFound => Ok(None),
        Err(e) => Err(git(e)),
    }
}

/// The oid `refs/joy/chats` points at, or `None` if unborn.
pub fn ref_target(root: &Path) -> Result<Option<Oid>, JoyError> {
    let repo = open_repo(root)?;
    match repo.refname_to_id(CHATS_REF) {
        Ok(oid) => Ok(Some(oid)),
        Err(e) if e.code() == ErrorCode::NotFound => Ok(None),
        Err(e) => Err(git(e)),
    }
}

/// Serialize a chat's metadata (everything but the message list).
fn meta_yaml(chat: &Chat) -> Result<String, JoyError> {
    let mut meta = chat.clone();
    meta.messages = Vec::new();
    Ok(serde_yaml_ng::to_string(&meta)?)
}

/// The stable storage id of a message (its own id, or the deterministic
/// synthetic one for pre-channel messages).
fn message_key(m: &ChatMessage) -> String {
    if m.id.is_empty() {
        m.synthetic_id()
    } else {
        m.id.clone()
    }
}

/// Build the `<chat-id>/` subtree (meta.yaml + messages/) and return its oid.
fn build_chat_tree(repo: &Repository, chat: &Chat) -> Result<Oid, JoyError> {
    let meta_blob = repo.blob(meta_yaml(chat)?.as_bytes()).map_err(git)?;
    let mut cb = repo.treebuilder(None).map_err(git)?;
    cb.insert(META_FILE, meta_blob, i32::from(FileMode::Blob))
        .map_err(git)?;

    if !chat.messages.is_empty() {
        let mut mb = repo.treebuilder(None).map_err(git)?;
        for m in &chat.messages {
            let blob = repo
                .blob(serde_yaml_ng::to_string(m)?.as_bytes())
                .map_err(git)?;
            mb.insert(
                format!("{}.yaml", message_key(m)),
                blob,
                i32::from(FileMode::Blob),
            )
            .map_err(git)?;
        }
        let messages_tree = mb.write().map_err(git)?;
        cb.insert(MESSAGES_DIR, messages_tree, i32::from(FileMode::Tree))
            .map_err(git)?;
    }
    cb.write().map_err(git)
}

/// Read a chat out of its `<chat-id>/` subtree (messages included). Does
/// NOT normalize — the semantic layer does that.
fn read_chat_tree(repo: &Repository, chat_tree: &Tree) -> Result<Option<Chat>, JoyError> {
    let Some(meta_entry) = chat_tree.get_name(META_FILE) else {
        return Ok(None);
    };
    let meta_blob = meta_entry
        .to_object(repo)
        .map_err(git)?
        .peel_to_blob()
        .map_err(git)?;
    let mut chat: Chat = serde_yaml_ng::from_slice(meta_blob.content())?;
    chat.messages.clear();
    if let Some(msgs_entry) = chat_tree.get_name(MESSAGES_DIR) {
        if let Ok(msgs_tree) = msgs_entry.to_object(repo).map_err(git)?.peel_to_tree() {
            for e in msgs_tree.iter() {
                let blob = e
                    .to_object(repo)
                    .map_err(git)?
                    .peel_to_blob()
                    .map_err(git)?;
                let m: ChatMessage = serde_yaml_ng::from_slice(blob.content())?;
                chat.messages.push(m);
            }
        }
    }
    Ok(Some(chat))
}

/// Read a chat by id out of a given root tree, if present.
fn read_chat_at(repo: &Repository, root_tree: &Tree, id: &str) -> Result<Option<Chat>, JoyError> {
    let Some(entry) = root_tree.get_name(id) else {
        return Ok(None);
    };
    let Ok(chat_tree) = entry.to_object(repo).map_err(git)?.peel_to_tree() else {
        return Ok(None);
    };
    read_chat_tree(repo, &chat_tree)
}

/// Commit `root_tree` onto `refs/joy/chats` with the given parent.
pub(crate) fn commit_root(
    repo: &Repository,
    parent: Option<&Commit>,
    root_tree: &Tree,
    message: &str,
) -> Result<Oid, JoyError> {
    let sig = signature(repo)?;
    let parents: Vec<&Commit> = parent.into_iter().collect();
    repo.commit(Some(CHATS_REF), &sig, &sig, message, root_tree, &parents)
        .map_err(git)
}

/// Load one chat by id from the ref, if present (un-normalized).
pub fn load_chat(root: &Path, id: &str) -> Result<Option<Chat>, JoyError> {
    let repo = open_repo(root)?;
    let Some(commit) = ref_commit(&repo)? else {
        return Ok(None);
    };
    let tree = commit.tree().map_err(git)?;
    read_chat_at(&repo, &tree, id)
}

/// Load every chat from the ref (un-normalized, unsorted).
pub fn load_chats(root: &Path) -> Result<Vec<Chat>, JoyError> {
    let repo = open_repo(root)?;
    let Some(commit) = ref_commit(&repo)? else {
        return Ok(Vec::new());
    };
    let tree = commit.tree().map_err(git)?;
    let mut chats = Vec::new();
    for entry in tree.iter() {
        let Some(name) = entry.name() else { continue };
        if let Some(chat) = read_chat_at(&repo, &tree, name)? {
            chats.push(chat);
        }
    }
    Ok(chats)
}

/// Upsert a chat onto the ref: splice its subtree into the root tree and
/// commit. The message blobs are content-addressed, so re-saving an
/// unchanged chat produces no new objects for its messages.
pub fn save_chat(root: &Path, chat: &Chat) -> Result<(), JoyError> {
    let repo = open_repo(root)?;
    let parent = ref_commit(&repo)?;
    let base_tree = match &parent {
        Some(c) => Some(c.tree().map_err(git)?),
        None => None,
    };
    let mut rb = repo.treebuilder(base_tree.as_ref()).map_err(git)?;
    let chat_tree = build_chat_tree(&repo, chat)?;
    rb.insert(&chat.id, chat_tree, i32::from(FileMode::Tree))
        .map_err(git)?;
    let root_tree_oid = rb.write().map_err(git)?;
    let root_tree = repo.find_tree(root_tree_oid).map_err(git)?;
    commit_root(
        &repo,
        parent.as_ref(),
        &root_tree,
        &format!("chat {} [no-item]", chat.id),
    )?;
    Ok(())
}

/// Remove a chat's whole subtree from the ref (garbage collection once
/// every human deleted it). A no-op if the chat is not on the ref.
pub fn remove_chat(root: &Path, id: &str) -> Result<(), JoyError> {
    let repo = open_repo(root)?;
    let Some(parent) = ref_commit(&repo)? else {
        return Ok(());
    };
    let tree = parent.tree().map_err(git)?;
    if tree.get_name(id).is_none() {
        return Ok(());
    }
    let mut rb = repo.treebuilder(Some(&tree)).map_err(git)?;
    rb.remove(id).map_err(git)?;
    let root_tree_oid = rb.write().map_err(git)?;
    let root_tree = repo.find_tree(root_tree_oid).map_err(git)?;
    commit_root(
        &repo,
        Some(&parent),
        &root_tree,
        &format!("delete chat {id} [no-item]"),
    )?;
    Ok(())
}

/// Merge a divergent `refs/joy/chats`: union each chat's messages by id
/// and three-way-merge its metadata, then write a merge commit with both
/// sides as parents and move the ref to it. Returns the merged oid.
///
/// Message union is conflict-free because a message id is immutable and
/// client-minted; only metadata (title, participants, deleted_for,
/// read_only) can diverge, and that goes through the field-level YAML
/// merge. A chat GC'd on one side but still present on the other is kept
/// with its delete marks and re-collected on the next GC pass — the
/// deleted_for marks are the source of truth, so this self-heals.
/// A named subtree of `parent`, if present.
fn named_tree<'a>(repo: &'a Repository, parent: &Tree<'a>, name: &str) -> Option<Tree<'a>> {
    parent
        .get_name(name)
        .and_then(|e| e.to_object(repo).ok())
        .and_then(|o| o.peel_to_tree().ok())
}

/// Whether a `<cid>/` subtree is the sealed new format (keys/ + log/); a
/// legacy chat carries `meta.yaml` instead.
fn subtree_is_new_format(chat_tree: &Tree) -> bool {
    chat_tree.get_name("keys").is_some() || chat_tree.get_name("log").is_some()
}

/// Union the entries of two leaf subtrees by name. Content-addressed
/// filenames mean identical names carry identical bytes, so the union is
/// keyless and conflict-free. `None` when both are absent.
fn union_leaf(
    repo: &Repository,
    a: Option<&Tree>,
    b: Option<&Tree>,
) -> Result<Option<Oid>, JoyError> {
    if a.is_none() && b.is_none() {
        return Ok(None);
    }
    let mut tb = repo.treebuilder(None).map_err(git)?;
    for t in [a, b].into_iter().flatten() {
        for e in t.iter() {
            if let Some(name) = e.name() {
                tb.insert(name, e.id(), e.filemode()).map_err(git)?;
            }
        }
    }
    Ok(Some(tb.write().map_err(git)?))
}

/// Keyless union of two sealed `<cid>/` subtrees (keys/ + log/). Never
/// decrypts; the forge, a seedless peer and the platform all produce the
/// identical merge. All chat-state resolution happens at read-time fold.
fn union_chat_subtrees(
    repo: &Repository,
    ours: Option<&Tree>,
    theirs: Option<&Tree>,
) -> Result<Oid, JoyError> {
    let keys = union_leaf(
        repo,
        ours.and_then(|t| named_tree(repo, t, "keys")).as_ref(),
        theirs.and_then(|t| named_tree(repo, t, "keys")).as_ref(),
    )?;
    let log = union_leaf(
        repo,
        ours.and_then(|t| named_tree(repo, t, "log")).as_ref(),
        theirs.and_then(|t| named_tree(repo, t, "log")).as_ref(),
    )?;
    let mut tb = repo.treebuilder(None).map_err(git)?;
    if let Some(k) = keys {
        tb.insert("keys", k, i32::from(FileMode::Tree))
            .map_err(git)?;
    }
    if let Some(l) = log {
        tb.insert("log", l, i32::from(FileMode::Tree))
            .map_err(git)?;
    }
    tb.write().map_err(git)
}

/// Reconcile the local [`CHATS_REF`] with an already-fetched
/// [`CHATS_TRACKING_REF`] (ADR JAPP-00DC-FC): adopt the remote ref when
/// none exists locally (a fresh clone carries no custom refs),
/// fast-forward when behind, or message-union merge (via [`merge_refs`])
/// when diverged. Pure local ref surgery — the caller owns fetch and
/// push (each transport plumbs credentials differently). Returns whether
/// the local ref now carries commits the remote still needs (push it).
///
/// This is THE decision tree every sync path runs; the platform server
/// and the desktop shell carry historical private copies that should
/// converge on this helper.
pub fn reconcile_with_tracking(root: &Path) -> Result<bool, JoyError> {
    let repo = open_repo(root)?;
    let local = repo.refname_to_id(CHATS_REF).ok();
    let remote = repo.refname_to_id(CHATS_TRACKING_REF).ok();
    match (local, remote) {
        (None, None) => Ok(false),
        (Some(_), None) => Ok(true), // only local: the remote needs it
        (None, Some(r)) => {
            repo.reference(CHATS_REF, r, true, "joy: adopt chats ref")
                .map_err(git)?;
            Ok(false)
        }
        (Some(l), Some(r)) if l == r => Ok(false),
        (Some(l), Some(r)) => {
            let base = repo.merge_base(l, r).ok();
            if base == Some(l) {
                // local behind: fast-forward, nothing to push back
                repo.reference(CHATS_REF, r, true, "joy: fast-forward chats ref")
                    .map_err(git)?;
                Ok(false)
            } else if base == Some(r) {
                Ok(true) // local ahead: push only
            } else {
                // diverged: union messages, three-way-merge metadata
                merge_refs(root, l, r)?;
                Ok(true)
            }
        }
    }
}

pub fn merge_refs(root: &Path, ours: Oid, theirs: Oid) -> Result<Oid, JoyError> {
    let repo = open_repo(root)?;
    let ours_c = repo.find_commit(ours).map_err(git)?;
    let theirs_c = repo.find_commit(theirs).map_err(git)?;
    let ours_tree = ours_c.tree().map_err(git)?;
    let theirs_tree = theirs_c.tree().map_err(git)?;
    let base_tree = match repo.merge_base(ours, theirs) {
        Ok(oid) => Some(repo.find_commit(oid).map_err(git)?.tree().map_err(git)?),
        Err(e) if e.code() == ErrorCode::NotFound => None,
        Err(e) => return Err(git(e)),
    };

    let mut ids: BTreeSet<String> = BTreeSet::new();
    for e in ours_tree.iter().chain(theirs_tree.iter()) {
        if let Some(name) = e.name() {
            ids.insert(name.to_string());
        }
    }

    let mut rb = repo.treebuilder(None).map_err(git)?;
    for id in &ids {
        let ours_ct = named_tree(&repo, &ours_tree, id);
        let theirs_ct = named_tree(&repo, &theirs_tree, id);
        // A sealed chat (keys/+log/) merges by pure keyless union of its
        // content-addressed sets; a legacy (meta.yaml) chat on either
        // side still takes the field-level merge until it is migrated.
        let sealed = ours_ct.as_ref().map(subtree_is_new_format).unwrap_or(true)
            && theirs_ct
                .as_ref()
                .map(subtree_is_new_format)
                .unwrap_or(true)
            && (ours_ct.is_some() || theirs_ct.is_some());
        if sealed {
            let oid = union_chat_subtrees(&repo, ours_ct.as_ref(), theirs_ct.as_ref())?;
            rb.insert(id, oid, i32::from(FileMode::Tree)).map_err(git)?;
            continue;
        }
        let ours_chat = read_chat_at(&repo, &ours_tree, id)?;
        let theirs_chat = read_chat_at(&repo, &theirs_tree, id)?;
        let base_chat = match &base_tree {
            Some(t) => read_chat_at(&repo, t, id)?,
            None => None,
        };
        let merged = match (ours_chat, theirs_chat) {
            (Some(o), Some(t)) => Some(merge_two_chats(base_chat.as_ref(), &o, &t)?),
            (Some(o), None) => Some(o),
            (None, Some(t)) => Some(t),
            (None, None) => None,
        };
        if let Some(chat) = merged {
            let oid = build_chat_tree(&repo, &chat)?;
            rb.insert(id, oid, i32::from(FileMode::Tree)).map_err(git)?;
        }
    }
    let root_tree_oid = rb.write().map_err(git)?;
    let root_tree = repo.find_tree(root_tree_oid).map_err(git)?;
    let sig = signature(&repo)?;
    // A merge commit has two parents, so neither is the current ref tip;
    // create it detached and force the ref onto it (writers are already
    // serialized by the caller's per-project lock).
    let oid = repo
        .commit(
            None,
            &sig,
            &sig,
            "merge chat refs [no-item]",
            &root_tree,
            &[&ours_c, &theirs_c],
        )
        .map_err(git)?;
    repo.reference(CHATS_REF, oid, true, "merge chat refs")
        .map_err(git)?;
    Ok(oid)
}

/// Three-way-merge two versions of one chat: union messages by id,
/// field-merge the metadata.
fn merge_two_chats(base: Option<&Chat>, ours: &Chat, theirs: &Chat) -> Result<Chat, JoyError> {
    let our_meta = meta_yaml(ours)?;
    let their_meta = meta_yaml(theirs)?;
    let merged_meta = match base {
        Some(b) => joy_core::merge::merge_yaml_doc(&meta_yaml(b)?, &our_meta, &their_meta)?,
        None if ours.updated >= theirs.updated => our_meta,
        None => their_meta,
    };
    let mut merged: Chat = serde_yaml_ng::from_str(&merged_meta)?;

    let mut seen = BTreeSet::new();
    let mut messages = Vec::new();
    for m in ours.messages.iter().chain(theirs.messages.iter()) {
        if seen.insert(message_key(m)) {
            messages.push(m.clone());
        }
    }
    merged.messages = messages;
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::chat::MessageKind;
    use chrono::{DateTime, Utc};
    use joy_core::member_ref::MemberRef;

    fn ts(sec: u32) -> DateTime<Utc> {
        format!("2026-07-05T00:00:{sec:02}Z").parse().unwrap()
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        Repository::init(dir.path()).unwrap();
        dir
    }

    fn msg(id: &str, sec: u32, text: &str) -> ChatMessage {
        ChatMessage {
            id: id.into(),
            at: ts(sec),
            author: MemberRef::new("a@x"),
            text: text.into(),
            kind: MessageKind::Text,
            delegated_by: None,
            turn_ms: None,
            tool_steps: None,
            tool: None,
            payload: None,
            details: None,
            enc: None,
            epoch: None,
        }
    }

    /// Move `refs/joy/chats` back to `oid` to fabricate a second divergent
    /// lane in the same repo (what two clients pushing concurrently do).
    fn reset_ref(root: &Path, oid: Oid) {
        let repo = Repository::discover(root).unwrap();
        repo.reference(CHATS_REF, oid, true, "test reset").unwrap();
    }

    /// Point the tracking ref at `oid`, as a fetch would.
    fn set_tracking(root: &Path, oid: Oid) {
        let repo = Repository::discover(root).unwrap();
        repo.reference(CHATS_TRACKING_REF, oid, true, "test fetch")
            .unwrap();
    }

    #[test]
    fn reconcile_covers_every_lane() {
        // absent everywhere: nothing to do, nothing to push
        let dir = repo();
        assert!(!reconcile_with_tracking(dir.path()).unwrap());

        // local only: the remote needs it
        let mut chat = Chat::new("c", vec![MemberRef::new("a@x")], ts(0));
        chat.messages.push(msg("m1", 1, "hello"));
        save_chat(dir.path(), &chat).unwrap();
        let first = ref_target(dir.path()).unwrap().unwrap();
        assert!(reconcile_with_tracking(dir.path()).unwrap());

        // in sync: no push
        set_tracking(dir.path(), first);
        assert!(!reconcile_with_tracking(dir.path()).unwrap());

        // local behind (tracking ahead): fast-forward, no push
        chat.messages.push(msg("m2", 2, "world"));
        save_chat(dir.path(), &chat).unwrap();
        let second = ref_target(dir.path()).unwrap().unwrap();
        reset_ref(dir.path(), first);
        set_tracking(dir.path(), second);
        assert!(!reconcile_with_tracking(dir.path()).unwrap());
        assert_eq!(ref_target(dir.path()).unwrap().unwrap(), second);

        // local ahead: push only, ref untouched
        set_tracking(dir.path(), first);
        assert!(reconcile_with_tracking(dir.path()).unwrap());
        assert_eq!(ref_target(dir.path()).unwrap().unwrap(), second);

        // diverged: message-union merge, then push
        let mut ours = chat.clone();
        ours.messages.push(msg("m3", 3, "ours"));
        save_chat(dir.path(), &ours).unwrap();
        let ours_oid = ref_target(dir.path()).unwrap().unwrap();
        reset_ref(dir.path(), second);
        let mut theirs = chat.clone();
        theirs.messages.push(msg("m4", 4, "theirs"));
        save_chat(dir.path(), &theirs).unwrap();
        set_tracking(dir.path(), ours_oid);
        assert!(reconcile_with_tracking(dir.path()).unwrap());
        let merged = load_chat(dir.path(), "c").unwrap().unwrap();
        let texts: Vec<_> = merged.messages.iter().map(|m| m.text.as_str()).collect();
        assert!(texts.contains(&"ours") && texts.contains(&"theirs"));

        // adopt: a fresh clone (no local ref) takes the tracking ref
        let fresh = repo();
        let donor = Repository::discover(dir.path()).unwrap();
        let target = Repository::discover(fresh.path()).unwrap();
        // copy the object into the fresh repo via a local fetch
        target
            .remote_anonymous(dir.path().to_str().unwrap())
            .unwrap()
            .fetch(&[&format!("+{CHATS_REF}:{CHATS_TRACKING_REF}")], None, None)
            .unwrap();
        drop(donor);
        assert!(!reconcile_with_tracking(fresh.path()).unwrap());
        assert!(load_chat(fresh.path(), "c").unwrap().is_some());
    }

    #[test]
    fn chat_ref_commits_carry_a_neutral_identity_not_the_writer() {
        // ADR JAPP-002A-30: a keyless reader of refs/joy/chats must not
        // learn WHO touched a chat. Even in a repo with a real developer
        // identity, every chat commit is authored+committed by the fixed
        // neutral identity with a day-coarsened time.
        let dir = repo();
        {
            let r = Repository::open(dir.path()).unwrap();
            let mut cfg = r.config().unwrap();
            cfg.set_str("user.name", "Horst Schwarz").unwrap();
            cfg.set_str("user.email", "horst.schwarz@joydev.com")
                .unwrap();
        }
        let mut chat = Chat::new("c1", vec![MemberRef::new("horst@example.com")], ts(0));
        chat.messages.push(msg("m1", 1, "secret"));
        save_chat(dir.path(), &chat).unwrap();

        let r = Repository::open(dir.path()).unwrap();
        let commit = r.find_commit(r.refname_to_id(CHATS_REF).unwrap()).unwrap();
        for sig in [commit.author(), commit.committer()] {
            assert_eq!(sig.name(), Some("joy"));
            assert_eq!(sig.email(), Some("joy@localhost"));
            assert_eq!(sig.when().seconds() % 86_400, 0, "day-coarsened time");
            let blob = format!("{} {}", sig.name().unwrap(), sig.email().unwrap());
            assert!(
                !blob.to_lowercase().contains("horst"),
                "writer identity leaked into the commit: {blob}"
            );
        }
    }

    #[test]
    fn round_trips_a_chat_with_messages() {
        let dir = repo();
        let mut chat = Chat::new("c", vec![MemberRef::new("a@x")], ts(0));
        chat.title = Some("T".into());
        chat.messages.push(msg("m1", 1, "hello"));
        chat.messages.push(msg("m2", 2, "world"));
        save_chat(dir.path(), &chat).unwrap();

        let loaded = load_chat(dir.path(), "c").unwrap().unwrap();
        assert_eq!(loaded.title.as_deref(), Some("T"));
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(load_chats(dir.path()).unwrap().len(), 1);

        remove_chat(dir.path(), "c").unwrap();
        assert!(load_chat(dir.path(), "c").unwrap().is_none());
    }

    #[test]
    fn diverging_message_appends_union_on_merge() {
        let dir = repo();
        let root = dir.path();
        let mut base = Chat::new("c", vec![MemberRef::new("a@x")], ts(0));
        base.messages.push(msg("m1", 1, "hello"));
        save_chat(root, &base).unwrap();
        let base_oid = ref_target(root).unwrap().unwrap();

        let mut ours = base.clone();
        ours.messages.push(msg("m2", 2, "ours"));
        ours.updated = ts(2);
        save_chat(root, &ours).unwrap();
        let ours_oid = ref_target(root).unwrap().unwrap();

        reset_ref(root, base_oid);
        let mut theirs = base.clone();
        theirs.messages.push(msg("m3", 3, "theirs"));
        theirs.updated = ts(3);
        save_chat(root, &theirs).unwrap();
        let theirs_oid = ref_target(root).unwrap().unwrap();

        merge_refs(root, ours_oid, theirs_oid).unwrap();
        let merged = load_chat(root, "c").unwrap().unwrap();
        let ids: BTreeSet<&str> = merged.messages.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(merged.messages.len(), 3);
        assert!(ids.contains("m1") && ids.contains("m2") && ids.contains("m3"));
    }

    #[test]
    fn diverging_metadata_three_way_merges() {
        let dir = repo();
        let root = dir.path();
        let base = Chat::new("c", vec![MemberRef::new("a@x")], ts(0));
        save_chat(root, &base).unwrap();
        let base_oid = ref_target(root).unwrap().unwrap();

        let mut ours = base.clone();
        ours.title = Some("Renamed".into());
        ours.updated = ts(2);
        save_chat(root, &ours).unwrap();
        let ours_oid = ref_target(root).unwrap().unwrap();

        reset_ref(root, base_oid);
        let mut theirs = base.clone();
        theirs.subtitle = Some("sub".into());
        theirs.updated = ts(1);
        save_chat(root, &theirs).unwrap();
        let theirs_oid = ref_target(root).unwrap().unwrap();

        merge_refs(root, ours_oid, theirs_oid).unwrap();
        let merged = load_chat(root, "c").unwrap().unwrap();
        assert_eq!(merged.title.as_deref(), Some("Renamed"));
        assert_eq!(merged.subtitle.as_deref(), Some("sub"));
    }

    #[test]
    fn diverging_mode_writes_union_per_delegator() {
        use joy_core::model::config::InteractionLevel;

        // Two chats derived from one base, each storing a level for a
        // DIFFERENT delegator under the SAME agent: the nested-map merge
        // must keep both entries (ADR JAPP-00F3-E8).
        let dir = repo();
        let root = dir.path();
        let base = Chat::new(
            "c",
            vec![
                MemberRef::new("a@x"),
                MemberRef::new("b@x"),
                MemberRef::new("ai:claude@joy"),
            ],
            ts(0),
        );
        save_chat(root, &base).unwrap();
        let base_oid = ref_target(root).unwrap().unwrap();

        let mut ours = base.clone();
        ours.interaction_levels
            .entry("ai:claude@joy".to_string())
            .or_default()
            .insert("a@x".to_string(), InteractionLevel::Confirmed);
        ours.updated = ts(2);
        save_chat(root, &ours).unwrap();
        let ours_oid = ref_target(root).unwrap().unwrap();

        reset_ref(root, base_oid);
        let mut theirs = base.clone();
        theirs
            .interaction_levels
            .entry("ai:claude@joy".to_string())
            .or_default()
            .insert("b@x".to_string(), InteractionLevel::Autonomous);
        theirs.updated = ts(1);
        save_chat(root, &theirs).unwrap();
        let theirs_oid = ref_target(root).unwrap().unwrap();

        merge_refs(root, ours_oid, theirs_oid).unwrap();
        let merged = load_chat(root, "c").unwrap().unwrap();
        assert_eq!(
            merged.interaction_level_override("ai:claude@joy", "a@x"),
            Some(InteractionLevel::Confirmed)
        );
        assert_eq!(
            merged.interaction_level_override("ai:claude@joy", "b@x"),
            Some(InteractionLevel::Autonomous)
        );
    }
}
