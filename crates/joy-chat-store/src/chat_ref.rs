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

use joy_chat::model::chat::{Chat, ChatMessage};
use joy_core::error::JoyError;

/// The dedicated ref chats live on. Outside `refs/heads/`, so it never
/// appears in `git log`, `git branch`, or a plain `git pull`.
pub const CHATS_REF: &str = "refs/joy/chats";

/// The local tracking ref a fetch of [`CHATS_REF`] lands on before the
/// reconcile (every sync path uses the same name, so a half-finished sync
/// never leaks state between callers).
pub const CHATS_TRACKING_REF: &str = "refs/joy/chats-remote";

/// How often a ref write retries when another writer moved the tip
/// first (JOY-023B-7E). Contention is short: the loser re-reads the new
/// tip and folds its work onto it.
pub(crate) const REF_MOVE_ATTEMPTS: usize = 8;

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

/// The stable storage id of a message (its own id, or the deterministic
/// synthetic one for pre-channel messages). Test-only since the legacy
/// writer left: the sealed store names its entries itself.
#[cfg(test)]
fn message_key(m: &ChatMessage) -> String {
    if m.id.is_empty() {
        m.synthetic_id()
    } else {
        m.id.clone()
    }
}

/// LEGACY READER — the sealing migration's eyes only. Reads the retired
/// plaintext `meta.yaml` + `messages/` layout so
/// `migrations::m_2026_07_sealed_chat_layout` can convert it; no product
/// surface reads this shape. Does NOT normalize.
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

/// How many chat writes pass between two maintenance checks.
const MAINTAIN_EVERY: usize = 64;
static WRITES_SINCE_MAINTENANCE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Let git tidy its object store now and then (JOY-023C-1E).
///
/// Every chat write is a commit, and these go through libgit2, which never
/// runs the auto-gc the git binary runs after its own commits. Nothing
/// else packed or pruned either, so a project only ever grew: the
/// operator's sandbox reached 39 MB of `.git` for 0.7 MiB of actual
/// content, in 6140 loose objects and not a single pack.
///
/// `--auto` means git decides whether there is anything worth doing, which
/// is why this can sit on the write path at all. Best effort throughout: a
/// missing git binary, a locked repo or a busy gc leave the store as it is
/// and the next write tries again.
fn maintain_occasionally(repo: &Repository) {
    use std::sync::atomic::Ordering;
    let n = WRITES_SINCE_MAINTENANCE.fetch_add(1, Ordering::Relaxed);
    if !n.is_multiple_of(MAINTAIN_EVERY) {
        return;
    }
    let git_dir = repo.path().to_path_buf();
    let _ = std::process::Command::new("git")
        .arg("--git-dir")
        .arg(&git_dir)
        .args(["gc", "--auto", "--quiet"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Commit `root_tree` onto `parent` and move the chats ref there, but ONLY
/// while the ref still points at `parent`. `Ok(None)` means it moved
/// underneath us and the caller has to redo its work on the new tip
/// (JOY-023B-7E).
///
/// Writing the commit with `repo.commit(Some(CHATS_REF), …)` looks like the
/// same thing and is not: libgit2 SETS the ref, it does not compare. Two
/// saves that read one tip then both commit a child of it, the second one
/// wins, and the first commit is orphaned TOGETHER WITH THE MESSAGES IT
/// CARRIED — silently. The desktop writes on every message, participant add
/// and read marker, plus a sync worker, so that race is ordinary traffic:
/// the operator's sandbox project had 505 of its 761 commits orphaned.
pub(crate) fn commit_root(
    repo: &Repository,
    parent: Option<&Commit>,
    root_tree: &Tree,
    message: &str,
) -> Result<Option<Oid>, JoyError> {
    let sig = signature(repo)?;
    let parents: Vec<&Commit> = parent.into_iter().collect();
    // No ref name here: the commit object first, the ref move separately
    // and conditionally.
    let oid = repo
        .commit(None, &sig, &sig, message, root_tree, &parents)
        .map_err(git)?;
    let moved = match parent {
        // The ref must still be exactly where the caller read it.
        Some(base) => repo
            .reference_matching(CHATS_REF, oid, true, base.id(), message)
            .is_ok(),
        // No parent means we believe the ref does not exist yet; creating
        // it non-forced fails if someone else got there first.
        None => repo.reference(CHATS_REF, oid, false, message).is_ok(),
    };
    if moved {
        maintain_occasionally(repo);
    }
    Ok(moved.then_some(oid))
}

/// LEGACY READER — see [`read_chat_tree`]; the migration's tests are the
/// only callers (the migration itself sweeps via [`load_chats`]).
#[cfg(test)]
pub(crate) fn load_chat(root: &Path, id: &str) -> Result<Option<Chat>, JoyError> {
    let repo = open_repo(root)?;
    let Some(commit) = ref_commit(&repo)? else {
        return Ok(None);
    };
    let tree = commit.tree().map_err(git)?;
    read_chat_at(&repo, &tree, id)
}

/// LEGACY READER — see [`read_chat_tree`]; the migration and its tests
/// are the only callers. A sealed chat has no `meta.yaml`, so this
/// answers exactly the unmigrated set.
pub(crate) fn load_chats(root: &Path) -> Result<Vec<Chat>, JoyError> {
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

/// Remove a chat's whole subtree from the ref (garbage collection once
/// every human deleted it). A no-op if the chat is not on the ref.
pub fn remove_chat(root: &Path, id: &str) -> Result<(), JoyError> {
    let repo = open_repo(root)?;
    for _ in 0..REF_MOVE_ATTEMPTS {
        if remove_chat_once(&repo, id)? {
            return Ok(());
        }
    }
    Err(JoyError::Git(
        "the chats ref kept moving while deleting; try again".into(),
    ))
}

fn remove_chat_once(repo: &Repository, id: &str) -> Result<bool, JoyError> {
    let Some(parent) = ref_commit(repo)? else {
        return Ok(true);
    };
    let tree = parent.tree().map_err(git)?;
    if tree.get_name(id).is_none() {
        return Ok(true);
    }
    let mut rb = repo.treebuilder(Some(&tree)).map_err(git)?;
    rb.remove(id).map_err(git)?;
    let root_tree_oid = rb.write().map_err(git)?;
    let root_tree = repo.find_tree(root_tree_oid).map_err(git)?;
    let moved = commit_root(
        repo,
        Some(&parent),
        &root_tree,
        &format!("delete chat {id} [no-item]"),
    )?;
    Ok(moved.is_some())
}

/// Merge a divergent `refs/joy/chats`: keyless union of every chat's
/// sealed subtrees, then a merge commit with both sides as parents.
/// Conflict-free by construction — the entries are content-addressed —
/// and key-free, so the forge, a seedless peer and the platform all
/// produce the identical merge. Semantic resolution (title, markers,
/// levels) happens at read-time event fold, never here. A chat GC'd on
/// one side but present on the other is kept and re-collected on the
/// next pass — the deleted_for marks are the source of truth.
/// A named subtree of `parent`, if present.
fn named_tree<'a>(repo: &'a Repository, parent: &Tree<'a>, name: &str) -> Option<Tree<'a>> {
    parent
        .get_name(name)
        .and_then(|e| e.to_object(repo).ok())
        .and_then(|o| o.peel_to_tree().ok())
}

/// Whether a `<cid>/` subtree is the sealed layout (keys/ + log/) — the
/// only layout a merge accepts; the retired plaintext layout is the
/// sealing migration's business.
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
        // Only the sealed layout merges. A plaintext subtree here means a
        // clone wrote with a pre-sealing build; merging it would need the
        // retired format back, so it is refused by name instead.
        let sealed = [&ours_ct, &theirs_ct]
            .into_iter()
            .filter_map(|t| t.as_ref())
            .all(subtree_is_new_format);
        if !sealed {
            return Err(JoyError::Git(format!(
                "chat {id} is in the retired pre-sealing layout; run the chat migration                  (joy update) on the clone that wrote it before syncing"
            )));
        }
        let oid = union_chat_subtrees(&repo, ours_ct.as_ref(), theirs_ct.as_ref())?;
        rb.insert(id, oid, i32::from(FileMode::Tree)).map_err(git)?;
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

/// TEST-ONLY legacy writer: commits a chat in the retired plaintext
/// layout (meta.yaml + messages/), the shape the sealing migration
/// converts. Product code never writes this; the fixtures that exercise
/// the migration and the merge refusal do.
#[cfg(test)]
pub(crate) fn save_legacy_chat_for_tests(root: &Path, chat: &Chat) {
    let repo = open_repo(root).unwrap();
    let parent = ref_commit(&repo).unwrap();
    let base_tree = parent.as_ref().map(|c| c.tree().unwrap());
    let mut meta = chat.clone();
    meta.messages = Vec::new();
    let meta_blob = repo
        .blob(serde_yaml_ng::to_string(&meta).unwrap().as_bytes())
        .unwrap();
    let mut cb = repo.treebuilder(None).unwrap();
    cb.insert(META_FILE, meta_blob, i32::from(FileMode::Blob))
        .unwrap();
    if !chat.messages.is_empty() {
        let mut mb = repo.treebuilder(None).unwrap();
        for m in &chat.messages {
            let blob = repo
                .blob(serde_yaml_ng::to_string(m).unwrap().as_bytes())
                .unwrap();
            mb.insert(
                format!("{}.yaml", message_key(m)),
                blob,
                i32::from(FileMode::Blob),
            )
            .unwrap();
        }
        cb.insert(MESSAGES_DIR, mb.write().unwrap(), i32::from(FileMode::Tree))
            .unwrap();
    }
    let chat_tree = cb.write().unwrap();
    let mut rb = repo.treebuilder(base_tree.as_ref()).unwrap();
    rb.insert(&chat.id, chat_tree, i32::from(FileMode::Tree))
        .unwrap();
    let root_tree = repo.find_tree(rb.write().unwrap()).unwrap();
    commit_root(&repo, parent.as_ref(), &root_tree, "legacy stub [no-item]")
        .unwrap()
        .expect("stub commit moves the ref");
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use joy_chat::model::chat::MessageKind;
    use joy_core::member_ref::MemberRef;

    fn ts(sec: u32) -> DateTime<Utc> {
        format!("2026-07-05T00:00:{sec:02}Z").parse().unwrap()
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        Repository::init(dir.path()).unwrap();
        dir
    }

    /// JOY-023B-7E: two writers that read the SAME tip must not overwrite
    /// each other. The second commit is refused, its caller retries, and
    /// nothing that was already on the ref disappears.
    #[test]
    fn a_second_writer_on_the_same_tip_is_refused_instead_of_winning() {
        let dir = repo();
        let repo = open_repo(dir.path()).unwrap();
        let empty = repo
            .find_tree(repo.treebuilder(None).unwrap().write().unwrap())
            .unwrap();

        // first writer: the ref is unborn, so it creates it
        let first = commit_root(&repo, None, &empty, "one [no-item]")
            .unwrap()
            .expect("the first writer moves the ref");
        assert_eq!(repo.refname_to_id(CHATS_REF).unwrap(), first);

        // second writer, still holding the pre-first view (unborn): refused
        assert!(
            commit_root(&repo, None, &empty, "two [no-item]")
                .unwrap()
                .is_none(),
            "a stale view must not create the ref a second time"
        );
        assert_eq!(repo.refname_to_id(CHATS_REF).unwrap(), first);

        // a writer that read the CURRENT tip moves it
        let base = repo.find_commit(first).unwrap();
        let third = commit_root(&repo, Some(&base), &empty, "three [no-item]")
            .unwrap()
            .expect("the up-to-date writer moves the ref");
        assert_eq!(repo.refname_to_id(CHATS_REF).unwrap(), third);

        // …and one that still holds the OLD tip does not
        assert!(
            commit_root(&repo, Some(&base), &empty, "four [no-item]")
                .unwrap()
                .is_none(),
            "a stale tip must not clobber the newer one"
        );
        assert_eq!(repo.refname_to_id(CHATS_REF).unwrap(), third);
    }

    /// Commit a sealed-layout chat subtree carrying the given log entry
    /// names. Union and reconcile care only about content-addressed
    /// NAMES, so a stub blob per name exercises exactly what they do.
    fn save_sealed_stub(root: &Path, id: &str, log_names: &[&str]) {
        let repo = open_repo(root).unwrap();
        let parent = ref_commit(&repo).unwrap();
        let base_tree = parent.as_ref().map(|c| c.tree().unwrap());
        let mut lb = repo.treebuilder(None).unwrap();
        for name in log_names {
            let blob = repo.blob(name.as_bytes()).unwrap();
            lb.insert(*name, blob, i32::from(FileMode::Blob)).unwrap();
        }
        let log = lb.write().unwrap();
        let mut cb = repo.treebuilder(None).unwrap();
        cb.insert("log", log, i32::from(FileMode::Tree)).unwrap();
        let chat_tree = cb.write().unwrap();
        let mut rb = repo.treebuilder(base_tree.as_ref()).unwrap();
        rb.insert(id, chat_tree, i32::from(FileMode::Tree)).unwrap();
        let root_tree = repo.find_tree(rb.write().unwrap()).unwrap();
        commit_root(&repo, parent.as_ref(), &root_tree, "chat stub [no-item]")
            .unwrap()
            .expect("stub commit moves the ref");
    }

    /// The log entry names of a chat on the current ref tip.
    fn log_names(root: &Path, id: &str) -> BTreeSet<String> {
        let repo = open_repo(root).unwrap();
        let commit = ref_commit(&repo).unwrap().unwrap();
        let tree = commit.tree().unwrap();
        let chat = named_tree(&repo, &tree, id).unwrap();
        let log = named_tree(&repo, &chat, "log").unwrap();
        log.iter()
            .filter_map(|e| e.name().map(str::to_string))
            .collect()
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
        save_sealed_stub(dir.path(), "c", &["m1"]);
        let first = ref_target(dir.path()).unwrap().unwrap();
        assert!(reconcile_with_tracking(dir.path()).unwrap());

        // in sync: no push
        set_tracking(dir.path(), first);
        assert!(!reconcile_with_tracking(dir.path()).unwrap());

        // local behind (tracking ahead): fast-forward, no push
        save_sealed_stub(dir.path(), "c", &["m1", "m2"]);
        let second = ref_target(dir.path()).unwrap().unwrap();
        reset_ref(dir.path(), first);
        set_tracking(dir.path(), second);
        assert!(!reconcile_with_tracking(dir.path()).unwrap());
        assert_eq!(ref_target(dir.path()).unwrap().unwrap(), second);

        // local ahead: push only, ref untouched
        set_tracking(dir.path(), first);
        assert!(reconcile_with_tracking(dir.path()).unwrap());
        assert_eq!(ref_target(dir.path()).unwrap().unwrap(), second);

        // diverged: keyless union merge, then push
        save_sealed_stub(dir.path(), "c", &["m1", "m2", "m3"]);
        let ours_oid = ref_target(dir.path()).unwrap().unwrap();
        reset_ref(dir.path(), second);
        save_sealed_stub(dir.path(), "c", &["m1", "m2", "m4"]);
        set_tracking(dir.path(), ours_oid);
        assert!(reconcile_with_tracking(dir.path()).unwrap());
        let names = log_names(dir.path(), "c");
        for n in ["m1", "m2", "m3", "m4"] {
            assert!(names.contains(n), "missing {n} in {names:?}");
        }

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
        assert!(!log_names(fresh.path(), "c").is_empty());
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
        save_sealed_stub(dir.path(), "c1", &["m1"]);

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
    fn the_legacy_reader_serves_only_the_migration_shape() {
        // The retired plaintext layout is readable ONLY through the
        // migration's reader, and a delete removes the subtree whatever
        // its layout.
        let dir = repo();
        let mut chat = Chat::new("c", vec![MemberRef::new("a@x")], ts(0));
        chat.title = Some("T".into());
        chat.messages.push(msg("m1", 1, "hello"));
        chat.messages.push(msg("m2", 2, "world"));
        save_legacy_chat_for_tests(dir.path(), &chat);

        let loaded = load_chat(dir.path(), "c").unwrap().unwrap();
        assert_eq!(loaded.title.as_deref(), Some("T"));
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(load_chats(dir.path()).unwrap().len(), 1);

        remove_chat(dir.path(), "c").unwrap();
        assert!(load_chat(dir.path(), "c").unwrap().is_none());

        // …and a SEALED chat is invisible to the legacy reader.
        save_sealed_stub(dir.path(), "s", &["e1"]);
        assert!(load_chat(dir.path(), "s").unwrap().is_none());
        assert!(load_chats(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn a_pre_sealing_chat_refuses_to_merge_by_name() {
        let dir = repo();
        let root = dir.path();
        save_sealed_stub(root, "c", &["m1"]);
        let base_oid = ref_target(root).unwrap().unwrap();

        save_sealed_stub(root, "c", &["m1", "m2"]);
        let ours_oid = ref_target(root).unwrap().unwrap();

        reset_ref(root, base_oid);
        let mut legacy = Chat::new("old", vec![MemberRef::new("a@x")], ts(0));
        legacy.messages.push(msg("m9", 9, "pre-sealing"));
        save_legacy_chat_for_tests(root, &legacy);
        let theirs_oid = ref_target(root).unwrap().unwrap();

        let err = merge_refs(root, ours_oid, theirs_oid).unwrap_err();
        assert!(
            err.to_string().contains("old")
                && err.to_string().contains("retired pre-sealing layout"),
            "{err}"
        );
    }
}
