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
use git2::{Commit, ErrorCode, FileMode, ObjectType, Oid, Repository, Signature, Time, Tree};

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

/// Pack and sweep this store when it is worth it (design D3.7).
///
/// Every chat write is a commit, and these go through libgit2, which
/// never runs the auto-gc the git binary runs after its own commits.
/// Nothing else packed or pruned either, so a project only ever grew: the
/// operator's sandbox reached 39 MB of `.git` for 0.7 MiB of actual
/// content, in 6140 loose objects and not a single pack, 72 percent of
/// them unreachable (JOY-023C-1E).
///
/// What used to stand here was `git gc --auto` in a child process. That
/// broke the git2-only rule, it was a hazard next to a shallow checkout
/// (an externally written commit-graph bypasses the shallow grafts), and
/// its "every 64th write in this process" counter said nothing about the
/// store: a CLI run is one process, so it fired on the first write every
/// time. [`joy_core::vcs::maintenance`] decides per checkout instead, on
/// the loose object estimate, the wall clock floor and the per-checkout
/// gate, and the grace window is the one for a checkout joy does not own,
/// because a person's own git may be writing objects joy cannot see.
///
/// The window is the one for a checkout joy does not own, on every host.
/// D3.7 allows 24 hours where joy is the sole writer (the platform's
/// project clone, a desktop only store), but the chat store cannot tell
/// the two apart from here: the same function serves a person's own
/// checkout and the platform's clone. The conservative window costs a
/// store 13 days of garbage it could have freed; the other mistake would
/// cost somebody else's objects. When the write path learns which kind
/// of store it is writing (P8), it selects `Options::owned_store()` and
/// nothing else about this changes.
///
/// Best effort throughout: a store that cannot be packed or swept stays
/// as it is and the next write tries again.
fn maintain_occasionally(repo: &Repository) {
    let _ = joy_core::vcs::maintenance::maintain_if_due(
        repo,
        &joy_core::vcs::maintenance::Options::foreign_checkout(),
    );
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
    // The swap needs the commit's id, so the object cannot be written
    // after it, but it can be left unwritten when the ref has ALREADY
    // moved, and taken back out when the swap loses anyway (D3.7). Both
    // halves matter: the losing attempt used to leave its commit and its
    // trees in the store for ever, and up to eight attempts per write is
    // how the sandbox got 505 orphans out of 761 commits.
    if !ref_is_where_the_caller_read_it(repo, parent) {
        return Ok(None);
    }
    let sig = signature(repo)?;
    let parents: Vec<&Commit> = parent.into_iter().collect();
    // No ref name here: the commit object first, the ref move separately
    // and conditionally. The object is built as a buffer and hashed
    // BEFORE it is written, because the discard below may only ever
    // remove an object THIS attempt created: chat commits carry a fixed
    // signature with a day-coarsened time, so two writers over the same
    // parent, tree and message produce the byte-identical commit, and
    // the other writer's copy is not joy's to unlink.
    let buffer = repo
        .commit_create_buffer(&sig, &sig, message, root_tree, &parents)
        .map_err(git)?;
    let predicted = Oid::hash_object(ObjectType::Commit, &buffer).map_err(git)?;
    let existed_before = repo.odb().map(|odb| odb.exists(predicted)).unwrap_or(true);
    let oid = repo
        .odb()
        .map_err(git)?
        .write(ObjectType::Commit, &buffer)
        .map_err(git)?;
    let created_here = !existed_before && oid == predicted;
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
        return Ok(Some(oid));
    }
    // Lost the race in the window between the check and the swap, so the
    // commit is nobody's. It is unlinked only under both of the
    // protections D3.7 names for a deletion outside the sweep: this
    // attempt created the object (nobody else's copy), and the live
    // history does not reach it (not the tip, not an ancestor of it).
    discard_lost_commit(repo, oid, created_here);
    Ok(None)
}

/// Take a lost commit back out of the store, under both protections.
/// `created_here` says this attempt wrote the object (another writer's
/// byte-identical copy is not joy's to remove), and the history query
/// says the chats ref does not reach it. Answers whether the object was
/// unlinked.
fn discard_lost_commit(repo: &Repository, oid: Oid, created_here: bool) -> bool {
    if !created_here || chats_history_reaches(repo, oid) {
        return false;
    }
    joy_core::vcs::maintenance::discard_loose_object(repo, oid)
}

/// Whether `refs/joy/chats` reaches this commit: its tip, or an ancestor
/// of its tip. Asked immediately before an object is discarded, and
/// answered conservatively: an error from the graph query reads as "yes,
/// it is reachable", because the cost of a wrong yes is a dead object the
/// sweep collects in 14 days and the cost of a wrong no is a chat
/// history that no longer walks.
fn chats_history_reaches(repo: &Repository, oid: Oid) -> bool {
    let Ok(tip) = repo.refname_to_id(CHATS_REF) else {
        return false;
    };
    tip == oid || repo.graph_descendant_of(tip, oid).unwrap_or(true)
}

/// Whether the ref still stands where the caller read it, which is the
/// precondition the compare-and-swap below enforces a second time. Asked
/// BEFORE the commit object is written so the ordinary lost race costs no
/// object at all; the swap still decides, because another writer can
/// arrive between the two.
fn ref_is_where_the_caller_read_it(repo: &Repository, parent: Option<&Commit>) -> bool {
    let current = repo.refname_to_id(CHATS_REF).ok();
    match (parent, current) {
        (Some(base), Some(now)) => base.id() == now,
        (None, None) => true,
        _ => false,
    }
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
        let Ok(name) = entry.name() else { continue };
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
            if let Ok(name) = e.name() {
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

// ---- forge sync, composed over the one git engine ----------------------
//
// The raw verbs live in joy_core::vcs::forge (JOY-0265-D7); this file owns
// only the CHAT semantics: what to fetch, when to reconcile, when the
// forge still needs our commits.

/// Bidirectional chat sync with the forge: fetch the remote chat ref into
/// the tracking ref, reconcile (adopt / fast-forward / message-union
/// merge — ONE decision tree for every venue), then push when the local
/// ref carries commits the forge still needs. Chats live on this ref,
/// never the working branch, so `git log`/`git pull` ignore them.
pub fn sync_with_forge(root: &Path, auth: &joy_core::vcs::forge::Auth) -> Result<(), JoyError> {
    // Push FIRST (JAPP-01A3-4A): after a write the forge is usually
    // strictly behind this checkout, so one roundtrip delivers and the
    // reader's poll sees it a second later. Only a rejected push means
    // the forge moved meanwhile — then the classic road: fetch,
    // reconcile (adopt / fast-forward / union), push again. The old
    // fetch-before-every-push paid a full extra roundtrip on the hot
    // path of every message.
    if open_repo(root)?.refname_to_id(CHATS_REF).is_ok()
        && joy_core::vcs::forge::push_ref(root, auth, CHATS_REF).is_ok()
    {
        return Ok(());
    }
    if pull_from_forge(root, auth)? {
        joy_core::vcs::forge::push_ref(root, auth, CHATS_REF)
            .map_err(|e| JoyError::Git(format!("chats push failed: {e}")))?;
    }
    Ok(())
}

/// ONE inbound poll pass (JAPP-01A3-4A / JP-00E6-3D), the same algorithm
/// on every host: a cheap remote-hash compare, a fetch+reconcile only
/// when the forge moved, and a healing push when the local ref still
/// carries commits the forge needs (an earlier offline write). Takes the
/// engine's per-checkout gate. Returns whether the LOCAL chats ref
/// moved — the caller announces that however its host does
/// (fs-notice on the desktop, the chat bus on the platform).
pub fn poll_once(root: &Path, auth: &joy_core::vcs::forge::Auth) -> Result<bool, JoyError> {
    // The cheap question first, OUTSIDE the checkout gate. The gate's own
    // contract (JP-00DB-61) is "every path that MOVES refs takes it;
    // reads stay lock-free", and a remote-hash compare moves nothing.
    // Holding it across the forge contact made every local write queue
    // behind a network round trip: 1.3 s over SSH, once per poll tick,
    // so a message, a delegation or a level took seconds to land
    // (JOY-027F-EB). The platform drew the same line on 2026-08-29
    // (JP-00F5-2A) in its own wrapper; here it is in the shared pass, so
    // both hosts have it.
    let before = ref_target(root)?.map(|oid| oid.to_string());
    let remote = remote_hash(root, auth)?;
    if remote != before {
        // Something moved: only now the refs move, only now the gate.
        let gate = joy_core::vcs::forge::checkout_gate(root);
        let _guard = gate.lock().unwrap_or_else(|e| e.into_inner());
        if pull_from_forge(root, auth)? {
            // best effort: delivery heals what an offline write left - but
            // only when the local ref carries what the forge lacks. After a
            // plain fast-forward local == remote, and a push then was one
            // forge contact for nothing: 2.3 s and a throttle slot at
            // Codeberg, twice per chat opened (JP-00FA-FF, 2026-08-29).
            let local = ref_target(root)?.map(|oid| oid.to_string());
            if local != remote {
                let _ = joy_core::vcs::forge::push_ref(root, auth, CHATS_REF);
            }
        }
    }
    let after = ref_target(root)?.map(|oid| oid.to_string());
    Ok(after != before)
}

/// Roots with a detached delivery in flight; the bool is "go one more
/// round": a write landing DURING a delivery marks it, and the running
/// thread loops once more instead of a second one piling onto the same
/// repository.
static DELIVERIES: std::sync::Mutex<Option<std::collections::HashMap<std::path::PathBuf, bool>>> =
    std::sync::Mutex::new(None);

/// Deliver the chats ref to the forge — DETACHED and coalescing, the
/// same mechanics on every host (JAPP-01A3-4A): the caller returns
/// right after its local commit, the roundtrip runs on its own thread
/// behind the per-checkout gate, and WHEN a write reaches the forge
/// stays best effort (JAPP-0126-E2) — a failed round only delays
/// visibility, the next write or poll heals it.
pub fn deliver_detached(root: std::path::PathBuf, auth: joy_core::vcs::forge::Auth) {
    {
        let mut guard = DELIVERIES.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(std::collections::HashMap::new);
        if let Some(again) = map.get_mut(&root) {
            // a delivery is running: it covers this write with one more round
            *again = true;
            return;
        }
        map.insert(root.clone(), false);
    }
    std::thread::spawn(move || loop {
        {
            let gate = joy_core::vcs::forge::checkout_gate(&root);
            let _guard = gate.lock().unwrap_or_else(|e| e.into_inner());
            if let Err(e) = sync_with_forge(&root, &auth) {
                eprintln!("joy: chats delivery failed (offline?): {e}");
            }
        }
        let mut guard = DELIVERIES.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(std::collections::HashMap::new);
        match map.get_mut(&root) {
            // a write landed while we were pushing: one more round
            Some(again) if *again => *again = false,
            _ => {
                map.remove(&root);
                break;
            }
        }
    });
}

/// Read-side chat refresh: fetch and reconcile, never push. Lets a read
/// recover the chat ref the working-branch clone left behind, and pick up
/// chats another writer pushed. Returns whether the local ref now carries
/// commits the forge still needs (a later [`sync_with_forge`] delivers
/// them).
pub fn pull_from_forge(root: &Path, auth: &joy_core::vcs::forge::Auth) -> Result<bool, JoyError> {
    fetch_from_forge(root, auth)?;
    reconcile_with_tracking(root)
}

/// The network half of [`pull_from_forge`] on its own: bring the forge's
/// chat ref into [`CHATS_TRACKING_REF`] and touch nothing else. A host
/// that serves readers while it syncs runs this WITHOUT the lock those
/// readers wait on, and takes the lock only for
/// [`reconcile_with_tracking`] (JP-0115-EC: a forge that does not answer
/// must not stall reading). Returns what [`fetch_ref`] returns: whether
/// the forge had the ref at all.
///
/// [`fetch_ref`]: joy_core::vcs::forge::fetch_ref
pub fn fetch_from_forge(root: &Path, auth: &joy_core::vcs::forge::Auth) -> Result<bool, JoyError> {
    joy_core::vcs::forge::fetch_ref(root, auth, CHATS_REF, CHATS_TRACKING_REF)
        .map_err(|e| JoyError::Git(format!("chats fetch failed: {e}")))
}

/// The oid the FORGE's chat ref points at, without fetching anything
/// (JP-008B-24: the poll compares hashes and fetches only on a change).
/// `None` when the forge has no chats yet.
pub fn remote_hash(
    root: &Path,
    auth: &joy_core::vcs::forge::Auth,
) -> Result<Option<String>, JoyError> {
    joy_core::vcs::forge::ls_remote_ref(root, auth, CHATS_REF)
        .map_err(|e| JoyError::Git(format!("chats ls-remote failed: {e}")))
}

/// Reconcile the local [`CHATS_REF`] with an already-fetched
/// [`CHATS_TRACKING_REF`] (ADR JAPP-00DC-FC): adopt the remote ref when
/// none exists locally (a fresh clone carries no custom refs),
/// fast-forward when behind, or message-union merge (via [`merge_refs`])
/// when diverged. Pure local ref surgery — the caller owns fetch and
/// push (each transport plumbs credentials differently). Returns whether
/// the local ref now carries commits the remote still needs (push it).
///
/// This is THE decision tree every sync path runs, via
/// [`sync_with_forge`] / [`pull_from_forge`] on every venue.
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
        if let Ok(name) = e.name() {
            ids.insert(name.to_string());
        }
    }

    let mut rb = repo.treebuilder(None).map_err(git)?;
    for id in &ids {
        let ours_ct = named_tree(&repo, &ours_tree, id);
        let theirs_ct = named_tree(&repo, &theirs_tree, id);
        let oid = match (&ours_ct, &theirs_ct) {
            // A chat only one side carries has nothing to unite: it
            // travels as it is, whatever its layout. A pre-sealing
            // subtree here is the writer's clone's business - its
            // migration seals or drops it whenever that clone next runs
            // `joy update`, which may be never. Refusing the whole merge
            // over it left every OTHER clone in an endless "not
            // reachable" retry about a chat it never touched.
            (Some(only), None) | (None, Some(only)) => only.id(),
            // Both sides know the chat and both are sealed: union.
            (Some(o), Some(t)) if subtree_is_new_format(o) && subtree_is_new_format(t) => {
                union_chat_subtrees(&repo, Some(o), Some(t))?
            }
            // One side sealed, the other still the pre-sealing plaintext
            // layout (a clone that has not migrated yet, or one that
            // cannot: a chat nobody can read any more). The sealed side
            // IS the chat; the plaintext copy is dropped rather than
            // merged, and the merge does not refuse over it. Operator
            // 2026-09-02: the desktop's `general` sat sealed against the
            // platform's unconvertible plaintext copy, endlessly.
            //
            // Dropping loses nothing a reader could have had: the
            // plaintext side is either the same conversation in the old
            // shape, or unreadable ciphertext. Both remain reachable
            // through that ref's own history in the clone that wrote
            // them - every write here commits onto the previous tip.
            (Some(o), Some(_)) if subtree_is_new_format(o) => o.id(),
            (Some(_), Some(t)) if subtree_is_new_format(t) => t.id(),
            // Plaintext on both sides has no union to speak of; two
            // pre-sealing writers must migrate first.
            (Some(_), Some(_)) => {
                return Err(JoyError::Git(format!(
                    "chat {id} is in the pre-sealing layout on both sides; run the chat migration (joy update) on the clone that wrote it before syncing"
                )));
            }
            (None, None) => unreachable!("ids come from the two trees"),
        };
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
            .filter_map(|e| e.name().ok().map(str::to_string))
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
            attempt: 0,
            parts: Vec::new(),
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
            assert_eq!(sig.name().ok(), Some("joy"));
            assert_eq!(sig.email().ok(), Some("joy@localhost"));
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

    /// A pre-sealing chat that only ONE side carries rides through the
    /// merge untouched: the clone that never wrote it must not sit in an
    /// endless retry over it, and it cannot fix it either - only the
    /// writer's own `joy update` seals or drops it, whenever that
    /// happens. Refusal is reserved for a chat BOTH sides know where one
    /// of them is still plaintext.
    #[test]
    fn a_one_sided_pre_sealing_chat_travels_through_the_merge() {
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

        let merged = merge_refs(root, ours_oid, theirs_oid).unwrap();
        let repo = open_repo(root).unwrap();
        let tree = repo.find_commit(merged).unwrap().tree().unwrap();
        // the sealed chat united, the legacy one carried verbatim
        let c = named_tree(&repo, &tree, "c").unwrap();
        assert_eq!(named_tree(&repo, &c, "log").unwrap().len(), 2);
        let old = named_tree(&repo, &tree, "old").unwrap();
        assert!(old.get_name(META_FILE).is_some());
        assert_eq!(load_chat(root, "old").unwrap().unwrap().messages.len(), 1);
    }

    /// One side sealed the chat, the other still carries the plaintext
    /// copy (a clone that has not migrated, or one that cannot): the
    /// sealed side is the chat, the plaintext copy is dropped, and the
    /// merge goes through. Either direction.
    #[test]
    fn a_sealed_side_wins_over_a_pre_sealing_copy() {
        for sealed_is_ours in [true, false] {
            let dir = repo();
            let root = dir.path();
            let mut legacy = Chat::new("old", vec![MemberRef::new("a@x")], ts(0));
            legacy.messages.push(msg("m9", 9, "pre-sealing"));
            save_legacy_chat_for_tests(root, &legacy);
            let base_oid = ref_target(root).unwrap().unwrap();

            remove_chat(root, "old").unwrap();
            save_sealed_stub(root, "old", &["m1"]);
            let sealed_oid = ref_target(root).unwrap().unwrap();

            reset_ref(root, base_oid);
            legacy.messages.push(msg("m10", 10, "still plaintext"));
            save_legacy_chat_for_tests(root, &legacy);
            let plain_oid = ref_target(root).unwrap().unwrap();

            let (ours, theirs) = if sealed_is_ours {
                (sealed_oid, plain_oid)
            } else {
                (plain_oid, sealed_oid)
            };
            let merged = merge_refs(root, ours, theirs).unwrap();
            let repo = open_repo(root).unwrap();
            let tree = repo.find_commit(merged).unwrap().tree().unwrap();
            let old = named_tree(&repo, &tree, "old").unwrap();
            assert!(subtree_is_new_format(&old), "sealed side wins");
            assert!(old.get_name(META_FILE).is_none());
            // the plaintext copy is gone from the merged tree, and the
            // legacy reader no longer answers for it
            assert!(named_tree(&repo, &old, MESSAGES_DIR).is_none());
        }
    }

    #[test]
    fn plaintext_on_both_sides_still_refuses_by_name() {
        let dir = repo();
        let root = dir.path();
        let mut legacy = Chat::new("old", vec![MemberRef::new("a@x")], ts(0));
        legacy.messages.push(msg("m9", 9, "pre-sealing"));
        save_legacy_chat_for_tests(root, &legacy);
        let base_oid = ref_target(root).unwrap().unwrap();
        legacy.messages.push(msg("m10", 10, "ours"));
        save_legacy_chat_for_tests(root, &legacy);
        let ours_oid = ref_target(root).unwrap().unwrap();
        reset_ref(root, base_oid);
        legacy.messages.pop();
        legacy.messages.push(msg("m11", 11, "theirs"));
        save_legacy_chat_for_tests(root, &legacy);
        let theirs_oid = ref_target(root).unwrap().unwrap();

        let err = merge_refs(root, ours_oid, theirs_oid).unwrap_err();
        assert!(
            err.to_string().contains("old") && err.to_string().contains("pre-sealing layout"),
            "{err}"
        );
    }

    /// What the sealing migration relies on when it drops a chat no build
    /// can read: the id leaves the live ref, its neighbours stay, and a
    /// second run is a no-op. The migration re-runs on every `joy update`
    /// of every old project, so "already gone" must be a quiet success,
    /// never an error.
    #[test]
    fn dropping_a_chat_takes_it_off_the_live_ref_and_is_idempotent() {
        let dir = repo();
        let root = dir.path();
        let mut legacy = Chat::new("old", vec![MemberRef::new("a@x")], ts(0));
        legacy.messages.push(msg("m9", 9, "unreadable"));
        save_legacy_chat_for_tests(root, &legacy);
        save_sealed_stub(root, "c", &["m1"]);

        remove_chat(root, "old").unwrap();
        assert!(load_chat(root, "old").unwrap().is_none());
        let repo = open_repo(root).unwrap();
        let live = repo
            .find_commit(ref_target(root).unwrap().unwrap())
            .unwrap();
        let tree = live.tree().unwrap();
        assert!(tree.get_name("old").is_none(), "dropped");
        assert!(tree.get_name("c").is_some(), "neighbour untouched");

        // second run: still gone, still no error
        remove_chat(root, "old").unwrap();
        assert!(load_chat(root, "old").unwrap().is_none());
    }

    // ---- maintenance of the store (design D3.7) ------------------------

    /// Loose objects in a store, counted the way the sweep enumerates
    /// them: the two-hex fanout directories under `objects/`.
    fn loose_count(root: &Path) -> usize {
        let objects = open_repo(root).unwrap().path().join("objects");
        let mut n = 0;
        for fanout in std::fs::read_dir(&objects).unwrap().flatten() {
            let name = fanout.file_name().to_string_lossy().to_string();
            if name.len() == 2 && name.bytes().all(|b| b.is_ascii_hexdigit()) {
                n += std::fs::read_dir(fanout.path()).unwrap().count();
            }
        }
        n
    }

    /// Every byte the object store occupies, loose and packed.
    fn store_bytes(root: &Path) -> u64 {
        fn walk(dir: &Path) -> u64 {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return 0;
            };
            entries
                .flatten()
                .map(|e| match e.file_type() {
                    Ok(t) if t.is_dir() => walk(&e.path()),
                    _ => e.metadata().map(|m| m.len()).unwrap_or(0),
                })
                .sum()
        }
        walk(&open_repo(root).unwrap().path().join("objects"))
    }

    /// Put every loose object in the store `age` into the past, which is
    /// what the operator's store looks like: the garbage there IS old.
    /// Without this the acceptance case could only be run with a zero
    /// grace window, which is a configuration joy never ships.
    fn backdate_every_loose_object(root: &Path, age: std::time::Duration) {
        let when = filetime::FileTime::from_system_time(std::time::SystemTime::now() - age);
        let objects = open_repo(root).unwrap().commondir().join("objects");
        for fanout in std::fs::read_dir(&objects).unwrap().flatten() {
            let name = fanout.file_name().to_string_lossy().to_string();
            if name.len() != 2 || !name.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            for object in std::fs::read_dir(fanout.path()).unwrap().flatten() {
                filetime::set_file_mtime(object.path(), when).unwrap();
            }
        }
    }

    /// One commit that nothing points at, with its own tree and blob:
    /// what a compare-and-swap that lost the race used to leave behind on
    /// every attempt, eight times per write in the worst case.
    fn lost_write(repo: &Repository, n: usize) {
        let blob = repo.blob(format!("lost write {n}").as_bytes()).unwrap();
        let mut tb = repo.treebuilder(None).unwrap();
        tb.insert("log", blob, i32::from(FileMode::Blob)).unwrap();
        let tree = repo.find_tree(tb.write().unwrap()).unwrap();
        let sig = signature(repo).unwrap();
        repo.commit(None, &sig, &sig, "lost [no-item]", &tree, &[])
            .unwrap();
    }

    /// The sweep of D3.7 on the shape this store actually has: chats on
    /// `refs/joy/chats`, which gets no reflog, plus the orphans of lost
    /// races. The orphans go, every chat stays readable, and the store
    /// shrinks.
    #[test]
    fn the_sweep_reclaims_lost_writes_and_every_chat_survives() {
        let dir = repo();
        let chats: Vec<String> = (0..20).map(|c| format!("chat-{c}")).collect();
        for chat in &chats {
            let names: Vec<String> = (0..10).map(|m| format!("{chat}-event-{m}")).collect();
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            save_sealed_stub(dir.path(), chat, &refs);
        }
        let repo = open_repo(dir.path()).unwrap();
        for n in 0..200 {
            lost_write(&repo, n);
        }
        // The shipped configuration, not a test-only window: the
        // orphans are aged past the 14 days the product asks for, the
        // way the operator's store is aged, and the run is the one joy
        // performs.
        backdate_every_loose_object(
            dir.path(),
            joy_core::vcs::maintenance::GRACE_FOREIGN_CHECKOUT + std::time::Duration::from_secs(60),
        );
        let before = loose_count(dir.path());

        let outcome = joy_core::vcs::maintenance::maintain(
            &repo,
            &joy_core::vcs::maintenance::Options::foreign_checkout(),
        )
        .unwrap();

        assert!(
            outcome.removed_unreferenced >= 200,
            "every lost write and its tree goes: {outcome:?}"
        );
        let after = loose_count(dir.path());
        assert!(after < before / 4, "the store shrinks: {before} -> {after}");
        // D3.7's acceptance, in the numbers it is written in. The loose
        // object number is comfortable rather than tight at this size (a
        // few hundred objects against 6700); the threshold itself is
        // exercised where the trigger is, in joy-core's own cases.
        assert!(after < 6700, "well under git's own loose object threshold");
        assert!(
            store_bytes(dir.path()) < 1_000_000,
            "a store of 20 chats stays below 1 MB"
        );
        // Every chat is still there, with every one of its events.
        for chat in &chats {
            let names = log_names(dir.path(), chat);
            assert_eq!(names.len(), 10, "{chat} lost events");
            assert!(names.contains(&format!("{chat}-event-7")));
        }
        // …and the ref's whole history is still walkable, which is what
        // a push to a forge needs.
        let tip = ref_target(dir.path()).unwrap().unwrap();
        let mut walk = repo.revwalk().unwrap();
        walk.push(tip).unwrap();
        assert_eq!(walk.count(), chats.len(), "one commit per chat, all there");
    }

    /// The discard after a lost swap is the one deletion in the package
    /// that bypasses the keep set and the grace window, so it carries
    /// both protections itself: only an object THIS attempt created, and
    /// only one the live chats history does not reach. The chat store
    /// signs with a fixed, day-coarsened signature, so two writers over
    /// the same parent, tree and message really do produce the same
    /// object id, and unlinking the winner's copy would break the
    /// revwalk, the push and every read.
    #[test]
    fn a_lost_commit_is_discarded_only_when_it_is_this_attempt_s_and_unreachable() {
        let dir = repo();
        save_sealed_stub(dir.path(), "general", &["one"]);
        save_sealed_stub(dir.path(), "general", &["two"]);
        let repo = open_repo(dir.path()).unwrap();
        let tip = ref_target(dir.path()).unwrap().unwrap();
        let ancestor = repo.find_commit(tip).unwrap().parent_id(0).unwrap();
        let objects = repo.commondir().join("objects");
        let path_of = |oid: Oid| {
            let hex = oid.to_string();
            let (prefix, rest) = hex.split_at(2);
            objects.join(prefix).join(rest)
        };

        // the tip and its ancestor: reachable, so never
        assert!(!discard_lost_commit(&repo, tip, true));
        assert!(path_of(tip).exists());
        assert!(!discard_lost_commit(&repo, ancestor, true));
        assert!(path_of(ancestor).exists());

        // an unreachable commit this attempt did NOT create: not joy's
        let sig = signature(&repo).unwrap();
        let empty = repo
            .find_tree(repo.treebuilder(None).unwrap().write().unwrap())
            .unwrap();
        let orphan = repo
            .commit(None, &sig, &sig, "somebody else's [no-item]", &empty, &[])
            .unwrap();
        assert!(!discard_lost_commit(&repo, orphan, false));
        assert!(path_of(orphan).exists());

        // and the case the rule is for
        assert!(discard_lost_commit(&repo, orphan, true));
        assert!(!path_of(orphan).exists());
    }

    /// The other half of D3.7's root-cause fix: a write that sees the ref
    /// already moved writes no object at all, so there is nothing for the
    /// sweep to reclaim later.
    #[test]
    fn a_refused_write_leaves_no_commit_behind() {
        let dir = repo();
        save_sealed_stub(dir.path(), "general", &["one"]);
        let repo = open_repo(dir.path()).unwrap();
        let stale = ref_commit(&repo).unwrap().unwrap();
        save_sealed_stub(dir.path(), "general", &["two"]);
        // the tree of the write that is about to be refused exists
        // either way; what must not appear is a commit on top of it
        let empty = repo
            .find_tree(repo.treebuilder(None).unwrap().write().unwrap())
            .unwrap();
        let before = loose_count(dir.path());

        // a writer still holding the older tip
        assert!(commit_root(&repo, Some(&stale), &empty, "stale [no-item]")
            .unwrap()
            .is_none());

        assert_eq!(
            loose_count(dir.path()),
            before,
            "a refused write must not cost the store an object"
        );
    }
}

#[cfg(test)]
mod forge_sync_tests {
    use super::*;

    /// A poll that finds the forge unchanged never touches the checkout
    /// gate (JOY-027F-EB). The test HOLDS the gate and polls from another
    /// thread: the old pass took the gate before asking the forge and
    /// would sit here forever; the pass that asks first comes straight
    /// back with "nothing moved".
    #[test]
    fn a_quiet_poll_never_takes_the_checkout_gate() {
        let base = std::env::temp_dir().join(format!("jp-chatref-quiet-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        let forge = base.join("forge.git");
        std::fs::create_dir_all(&forge).unwrap();
        git2::Repository::init_bare(&forge).unwrap();
        let clone = base.join("clone");
        let repo = git2::Repository::init(&clone).unwrap();
        repo.remote("origin", forge.to_str().unwrap()).unwrap();
        // No chats ref on either side: the forge answers "none", the
        // local side has "none", nothing moved.
        let gate = joy_core::vcs::forge::checkout_gate(&clone);
        let held = gate.lock().unwrap();
        let root = clone.clone();
        let polled =
            std::thread::spawn(move || poll_once(&root, &joy_core::vcs::forge::Auth::token("x")));
        let started = std::time::Instant::now();
        while !polled.is_finished() {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(10),
                "poll_once waited for the checkout gate although the forge had not moved"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        drop(held);
        assert!(
            !polled.join().unwrap().unwrap(),
            "nothing moved, so nothing changed"
        );
        std::fs::remove_dir_all(&base).ok();
    }

    /// Chats sync on their own ref (refs/joy/chats), never the working
    /// branch: write a chat, sync it, and see the message land on
    /// refs/joy/chats on the bare forge while the branch stays chat-free.
    #[test]
    fn a_rejected_first_push_falls_back_to_union_and_delivers() {
        // Push-first (JAPP-01A3-4A): when the forge moved meanwhile, the
        // optimistic push is rejected and the classic road must still
        // deliver — fetch, union-reconcile, push. Both writers' messages
        // end up on the forge, nothing is lost.
        let base = std::env::temp_dir().join(format!("jp-chatref-race-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        let forge = base.join("forge.git");
        std::fs::create_dir_all(&forge).unwrap();
        git2::Repository::init_bare(&forge).unwrap();

        let seed = base.join("seed");
        let seed_repo = git2::Repository::init(&seed).unwrap();
        std::fs::create_dir_all(seed.join(".joy")).unwrap();
        let mut project = joy_core::model::Project::new("T".to_string(), Some("T".to_string()));
        let mut m = joy_core::model::project::Member::new(
            joy_core::model::project::MemberCapabilities::All,
        );
        m.verify_key = Some(
            joy_core::auth::IdentityKeypair::from_seed(&[5u8; 32])
                .public_key()
                .to_hex(),
        );
        project.register_member("horst@example.com", m).unwrap();
        joy_core::store::write_yaml(&seed.join(".joy/project.yaml"), &project).unwrap();
        crate::writer::set_thread_seed(Some(Some([5u8; 32])));
        let mut index = seed_repo.index().unwrap();
        index
            .add_all(["."], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = seed_repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("Seed", "seed@example.com").unwrap();
        seed_repo
            .commit(Some("HEAD"), &sig, &sig, "seed", &tree, &[])
            .unwrap();
        seed_repo.remote("origin", forge.to_str().unwrap()).unwrap();
        let branch = seed_repo.head().unwrap().shorthand().unwrap().to_string();
        seed_repo
            .find_remote("origin")
            .unwrap()
            .push(
                &[format!("refs/heads/{branch}:refs/heads/{branch}").as_str()],
                None,
            )
            .unwrap();
        let auth = joy_core::vcs::forge::Auth::token("x");

        // both sides clone BEFORE any chat exists
        let a = base.join("a");
        let b = base.join("b");
        joy_core::vcs::forge::clone(forge.to_str().unwrap(), &auth, &a).expect("clone a");
        joy_core::vcs::forge::clone(forge.to_str().unwrap(), &auth, &b).expect("clone b");

        let now = chrono::Utc::now();
        // A writes and syncs: the forge now holds A's ref
        let mut chat_a = crate::chats::ensure_general(&a, now).unwrap();
        crate::chats::append_message(
            &a,
            &mut chat_a,
            joy_core::member_ref::MemberRef::new("horst@example.com"),
            "from a",
            now,
        )
        .unwrap();
        sync_with_forge(&a, &auth).expect("a sync");

        // B, unaware, writes its own chat: B's optimistic push is
        // rejected (the forge moved), the fallback unions and delivers
        let mut chat_b = crate::chats::ensure_general(&b, now).unwrap();
        crate::chats::append_message(
            &b,
            &mut chat_b,
            joy_core::member_ref::MemberRef::new("horst@example.com"),
            "from b",
            now,
        )
        .unwrap();
        sync_with_forge(&b, &auth).expect("b sync unions");

        // A pulls: BOTH messages are readable in A's checkout
        pull_from_forge(&a, &auth).expect("a pull");
        let general = crate::chat_store::load(&a, "general", &[5u8; 32])
            .unwrap()
            .expect("general opens");
        let texts: Vec<_> = general.messages.iter().map(|m| m.text.as_str()).collect();
        assert!(texts.contains(&"from a"), "{texts:?}");
        assert!(texts.contains(&"from b"), "{texts:?}");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn chats_ref_syncs_to_the_forge_off_the_branch() {
        let base = std::env::temp_dir().join(format!("jp-chatref-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        let forge = base.join("forge.git");
        std::fs::create_dir_all(&forge).unwrap();
        git2::Repository::init_bare(&forge).unwrap();

        let seed = base.join("seed");
        let seed_repo = git2::Repository::init(&seed).unwrap();
        std::fs::create_dir_all(seed.join(".joy")).unwrap();
        std::fs::write(seed.join(".joy/marker"), "hi").unwrap();
        // chats are always sealed now: the writing member needs an
        // identity, and this thread needs its seed
        let mut project = joy_core::model::Project::new("T".to_string(), Some("T".to_string()));
        let mut m = joy_core::model::project::Member::new(
            joy_core::model::project::MemberCapabilities::All,
        );
        m.verify_key = Some(
            joy_core::auth::IdentityKeypair::from_seed(&[5u8; 32])
                .public_key()
                .to_hex(),
        );
        project.register_member("horst@example.com", m).unwrap();
        joy_core::store::write_yaml(&seed.join(".joy/project.yaml"), &project).unwrap();
        crate::writer::set_thread_seed(Some(Some([5u8; 32])));
        let mut index = seed_repo.index().unwrap();
        index
            .add_all(["."], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = seed_repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("Seed", "seed@example.com").unwrap();
        seed_repo
            .commit(Some("HEAD"), &sig, &sig, "seed", &tree, &[])
            .unwrap();
        seed_repo.remote("origin", forge.to_str().unwrap()).unwrap();
        let branch = seed_repo.head().unwrap().shorthand().unwrap().to_string();
        seed_repo
            .find_remote("origin")
            .unwrap()
            .push(
                &[format!("refs/heads/{branch}:refs/heads/{branch}").as_str()],
                None,
            )
            .unwrap();

        let checkout = base.join("checkout");
        joy_core::vcs::forge::clone(
            forge.to_str().unwrap(),
            &joy_core::vcs::forge::Auth::token("x"),
            &checkout,
        )
        .expect("clone");

        // write a chat onto refs/joy/chats, then sync just that ref
        let now = chrono::Utc::now();
        let mut chat = crate::chats::ensure_general(&checkout, now).unwrap();
        crate::chats::append_message(
            &checkout,
            &mut chat,
            joy_core::member_ref::MemberRef::new("horst@example.com"),
            "warp core stable",
            now,
        )
        .unwrap();
        sync_with_forge(&checkout, &joy_core::vcs::forge::Auth::token("x")).expect("chats sync");

        // the forge has refs/joy/chats with the message blob
        let forge_repo = git2::Repository::open_bare(&forge).unwrap();
        let chats_tree = forge_repo
            .find_reference(crate::chat_ref::CHATS_REF)
            .expect("chats ref on forge")
            .peel_to_commit()
            .unwrap()
            .tree()
            .unwrap();
        let general = chats_tree
            .get_name("general")
            .expect("general chat")
            .to_object(&forge_repo)
            .unwrap()
            .peel_to_tree()
            .unwrap();
        // sealed shape on the forge: keys/ + log/, no messages dir, and
        // the log blobs do NOT contain the plaintext
        assert!(general.get_name("messages").is_none(), "no plaintext dir");
        let log = general
            .get_name("log")
            .expect("log dir")
            .to_object(&forge_repo)
            .unwrap()
            .peel_to_tree()
            .unwrap();
        assert!(log.iter().count() > 0, "sealed events reached the forge");
        assert!(
            log.iter().all(|e| {
                let blob = e.to_object(&forge_repo).unwrap().peel_to_blob().unwrap();
                !String::from_utf8_lossy(blob.content()).contains("warp core stable")
            }),
            "the forge holds envelopes, not text"
        );

        // the working branch carries NO chats — they left it
        let head_tree = forge_repo
            .find_reference(&format!("refs/heads/{branch}"))
            .unwrap()
            .peel_to_tree()
            .unwrap();
        if let Some(joy) = head_tree.get_name(".joy") {
            let joy_tree = joy.to_object(&forge_repo).unwrap().peel_to_tree().unwrap();
            assert!(
                joy_tree.get_name("chats").is_none(),
                "no .joy/chats on the working branch"
            );
        }

        std::fs::remove_dir_all(&base).ok();
    }
}
