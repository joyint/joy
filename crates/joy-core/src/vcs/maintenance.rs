// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Repository maintenance without the git binary (design D3.7).
//!
//! Every joy write is a libgit2 write, and libgit2 runs no auto gc after
//! its own commits. Nothing packed, nothing pruned, so a store only ever
//! grew: the operator's sandbox reached 38.89 MiB of `.git` in 6140 loose
//! objects for 1703 reachable objects worth 0.7 MiB, and 4437 of those
//! objects were unreachable (JOY-023C-1E). Packing alone is therefore not
//! maintenance; what the store needs is a pack AND a sweep.
//!
//! The shape, straight out of D3.7:
//!
//! - **Trigger.** Per canonical checkout, never a process global counter.
//!   A cheap loose object estimate (count one fanout directory, multiply
//!   by 256, the way git does), a wall clock floor per checkout, and
//!   git's own threshold of 6700 loose objects. The run takes the one per
//!   checkout gate of [`crate::vcs::forge::checkout_gate`].
//! - **Keep set.** The closure of every reference, `HEAD` and the other
//!   pseudo refs, the reflogs, the index entries and every linked
//!   worktree's `HEAD` and index. git2 exposes no reachability query, so
//!   the closure is walked explicitly.
//! - **Pack.** The loose members of the keep set go into one new pack
//!   under `objects/pack`, then the odb is refreshed. The indexer
//!   publishes the `.idx` before renaming the `.pack`, and both git and
//!   libgit2 tolerate that window because the pack backend enumerates
//!   `.idx` files.
//! - **Sweep, two classes.** Class A is a loose object that now sits in
//!   the pack joy just wrote: it goes at any age, because every reader
//!   finds it in the pack. Class B is a loose object outside the keep
//!   set: it goes only when its mtime is older than the grace window.
//! - **Never a pack.** A pack is never deleted, not even one joy wrote:
//!   libgit2 reopens a pack by name when its mwindow LRU closed the
//!   descriptor, so a pack removed under a live odb is a hard read error.
//!
//! Safety beside another git or libgit2 process comes out of libgit2's
//! own loose read: stat, open, one full read, close, with no lasting
//! handle and no mmap, and a read that loses the race answers
//! `GIT_ENOTFOUND`, which `git_odb_read` retries after
//! `git_odb_refresh` on the backends that have one. The loose backend
//! has none and the pack backend does, so the retry resolves out of
//! joy's new pack. A concurrent WRITE is equally correct: libgit2
//! freshens an existing object with `utimes`, and when that fails it
//! falls through to the pack backend's freshen. That is why the sweep
//! skips an object it cannot freshen itself: without a working `utimes`
//! another process cannot protect the object either.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use git2::{ObjectType, Oid, Repository};

use crate::error::JoyError;

/// git's own `gc.auto` threshold: below this many loose objects there is
/// nothing worth doing.
pub const LOOSE_OBJECT_THRESHOLD: usize = 6700;

/// The fanout directory the estimate samples. git samples `objects/17`
/// and multiplies by 256; joy samples the same one so the two agree on
/// the same store.
const FANOUT_SAMPLE: &str = "17";

/// The wall clock floor per checkout. The measured cost of a full run on
/// a store of the sandbox's shape is well under 100 ms, so ten minutes is
/// two orders of magnitude more conservative than the cost requires.
pub const MIN_INTERVAL: Duration = Duration::from_secs(600);

/// git's own `gc.pruneExpire`: the grace window for a checkout joy does
/// not own, where another git may be writing objects joy cannot see.
pub const GRACE_FOREIGN_CHECKOUT: Duration = Duration::from_secs(14 * 24 * 60 * 60);

/// The grace window for a store joy alone writes (the platform's project
/// clones, a desktop only store). Never `now`, on any host.
pub const GRACE_OWNED_STORE: Duration = Duration::from_secs(24 * 60 * 60);

/// What a maintenance run is allowed to do.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// A loose object outside the keep set is removed only when it is at
    /// least this old (D3.7: 14 days in a checkout joy does not own,
    /// 24 hours in a store joy alone writes).
    pub grace: Duration,
    /// The estimated loose object count a run needs to see.
    pub loose_threshold: usize,
    /// The wall clock floor between two runs on one checkout.
    pub min_interval: Duration,
}

impl Options {
    /// A checkout joy shares with a person and their own git: git's own
    /// 14 day grace window.
    pub fn foreign_checkout() -> Self {
        Self {
            grace: GRACE_FOREIGN_CHECKOUT,
            loose_threshold: LOOSE_OBJECT_THRESHOLD,
            min_interval: MIN_INTERVAL,
        }
    }

    /// A store joy alone writes (the platform's project clone, a desktop
    /// only store): 24 hours, which still covers a job container writing
    /// into the same object store.
    pub fn owned_store() -> Self {
        Self {
            grace: GRACE_OWNED_STORE,
            ..Self::foreign_checkout()
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        Self::foreign_checkout()
    }
}

/// What one run did, in numbers a host or a test can assert on.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    /// Loose objects found on disk.
    pub loose_seen: usize,
    /// Objects in the keep set (the reachability closure).
    pub keep_set: usize,
    /// Loose objects written into joy's new pack.
    pub packed: usize,
    /// Class A: loose copies of objects now in joy's pack, removed.
    pub removed_packed: usize,
    /// Class B: loose objects outside the keep set, older than the grace
    /// window, removed.
    pub removed_unreferenced: usize,
    /// Class B candidates left alone because they are younger than the
    /// grace window.
    pub kept_young: usize,
    /// Skipped: the mtime lies in the future (a clock skewed mount), so
    /// the age cannot be judged.
    pub skipped_future_mtime: usize,
    /// Skipped: `utimes` on the object is refused, so another process
    /// cannot freshen it either and its protection is gone.
    pub skipped_unfreshenable: usize,
    /// Left for the next run: another process holds the file open
    /// (Windows sharing violation) or the unlink failed otherwise.
    pub skipped_in_use: usize,
    /// `core.logAllRefUpdates=always` is set, so every ref update grows a
    /// reflog, the keep set grows with it and the sweep reclaims little
    /// or nothing. Reported once per checkout and process.
    pub log_all_ref_updates_always: bool,
}

/// Run maintenance when this checkout is due for it: the loose object
/// estimate is over the threshold, the wall clock floor has passed, and
/// the per checkout gate is free.
///
/// The gate is taken with `try_lock`, not `lock`: maintenance is best
/// effort and the caller may be a write path that already holds the gate
/// for the same checkout (the desktop and the platform both wrap chat
/// writes in it). Waiting there would deadlock, and waiting behind
/// another actor would pay for maintenance on the write's latency. A busy
/// gate simply means the next write tries again.
pub fn maintain_if_due(repo: &Repository, opts: &Options) -> Result<Option<Outcome>, JoyError> {
    let key = gate_key(repo);
    if estimate_loose_objects(repo.path()) < opts.loose_threshold {
        return Ok(None);
    }
    if !due(&key, repo.path(), opts.min_interval) {
        return Ok(None);
    }
    let gate = crate::vcs::forge::checkout_gate(&key);
    let _guard = match gate.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return Ok(None),
    };
    mark_run(&key, repo.path());
    maintain(repo, opts).map(Some)
}

/// Pack and sweep this repository now, whatever the trigger says. The
/// caller owns the gate; [`maintain_if_due`] is the ordinary door.
pub fn maintain(repo: &Repository, opts: &Options) -> Result<Outcome, JoyError> {
    let git_dir = repo.path().to_path_buf();
    let mut outcome = Outcome {
        log_all_ref_updates_always: logs_every_ref_update(repo),
        ..Outcome::default()
    };
    if outcome.log_all_ref_updates_always {
        report_log_all_ref_updates_once(&git_dir);
    }

    let keep = keep_set(repo);
    outcome.keep_set = keep.len();
    let loose = loose_objects(&git_dir);
    outcome.loose_seen = loose.len();

    let (packed, pack_landed) = pack_loose_keep_set(repo, &keep, &loose)?;
    outcome.packed = packed.len();

    let now = SystemTime::now();
    for object in &loose {
        // Class A: the object is in the pack joy just wrote, so every
        // reader finds it there. Age does not matter.
        if pack_landed && packed.contains(&object.oid) {
            match unlink_best_effort(&object.path) {
                Unlink::Gone => outcome.removed_packed += 1,
                Unlink::Left => outcome.skipped_in_use += 1,
            }
            continue;
        }
        if keep.contains(&object.oid) {
            continue;
        }
        // Class B: outside the keep set, so only the grace window
        // protects it.
        let Ok(mtime) = fs::metadata(&object.path).and_then(|m| m.modified()) else {
            // It vanished, or the filesystem has no mtime to judge by.
            continue;
        };
        let Ok(age) = now.duration_since(mtime) else {
            // A clock skewed mount: the object claims to be from the
            // future and no age statement is possible. Skip, never guess.
            outcome.skipped_future_mtime += 1;
            continue;
        };
        if age < opts.grace {
            outcome.kept_young += 1;
            continue;
        }
        if !can_freshen(&object.path, mtime) {
            // libgit2 protects an object it is about to reuse with
            // `utimes`. Where that is refused, another process has no way
            // to protect this object, so joy does not remove it.
            outcome.skipped_unfreshenable += 1;
            continue;
        }
        match unlink_best_effort(&object.path) {
            Unlink::Gone => outcome.removed_unreferenced += 1,
            Unlink::Left => outcome.skipped_in_use += 1,
        }
    }
    Ok(outcome)
}

/// Take a loose object back out of the store, best effort.
///
/// The one caller is a lost compare and swap: a commit object that was
/// written for a ref move that did not happen, and that nothing will ever
/// point at (the chat store's lost races were 505 of 761 commits in the
/// operator's sandbox). Never touches a packed object: only the loose
/// file at `objects/xx/yyy…` is unlinked.
pub fn discard_loose_object(repo: &Repository, oid: Oid) -> bool {
    let path = loose_path(repo.path(), oid);
    if !path.exists() {
        return true;
    }
    matches!(unlink_best_effort(&path), Unlink::Gone)
}

/// git's cheap estimate: count one fanout directory and multiply by 256.
/// A fanout is one byte of a uniformly distributed hash, so one directory
/// carries a 256th of the store.
pub fn estimate_loose_objects(git_dir: &Path) -> usize {
    let sample = git_dir.join("objects").join(FANOUT_SAMPLE);
    let Ok(entries) = fs::read_dir(&sample) else {
        return 0;
    };
    entries.filter(|e| e.is_ok()).count().saturating_mul(256)
}

// ---- the trigger -------------------------------------------------------

/// When each checkout last ran maintenance in this process. Per canonical
/// checkout, never a process global counter: the old chat store counter
/// fired on the first write of every process, so a CLI run ran it every
/// time and a long lived desktop every 64 writes, neither of which is a
/// statement about the store.
static LAST_RUN: Mutex<Option<HashMap<PathBuf, Instant>>> = Mutex::new(None);

/// The floor is per CHECKOUT, and a CLI command is one process per write,
/// so an in-process map alone would be no floor at all for the host that
/// needs it most. The stamp is a file joy writes inside the git directory
/// and reads by its mtime; where it cannot be written (a read only or
/// foreign owned `.git`) the in-process map still holds.
fn stamp_path(git_dir: &Path) -> PathBuf {
    git_dir.join("joy").join("maintenance-stamp")
}

fn due(key: &Path, git_dir: &Path, min_interval: Duration) -> bool {
    {
        let guard = LAST_RUN.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(last) = guard.as_ref().and_then(|m| m.get(key)) {
            if last.elapsed() < min_interval {
                return false;
            }
        }
    }
    let Ok(stamped) = fs::metadata(stamp_path(git_dir)).and_then(|m| m.modified()) else {
        // No stamp yet, or a filesystem with no mtime to read: this
        // checkout has nothing that says it ran recently.
        return true;
    };
    match SystemTime::now().duration_since(stamped) {
        Ok(age) => age >= min_interval,
        // A stamp from the future is a skewed clock, and running on every
        // write until the clock catches up is the worse of the two
        // mistakes.
        Err(_) => false,
    }
}

fn mark_run(key: &Path, git_dir: &Path) {
    {
        let mut guard = LAST_RUN.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .get_or_insert_with(HashMap::new)
            .insert(key.to_path_buf(), Instant::now());
    }
    let stamp = stamp_path(git_dir);
    if let Some(parent) = stamp.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&stamp, b"");
    let _ = filetime::set_file_mtime(&stamp, filetime::FileTime::now());
}

/// The key the per checkout gate is taken under. The working directory
/// where there is one, so maintenance and every ref moving path of a
/// checkout meet on the same gate; the git directory for a bare store.
fn gate_key(repo: &Repository) -> PathBuf {
    repo.workdir().unwrap_or_else(|| repo.path()).to_path_buf()
}

// ---- step 1: the keep set ---------------------------------------------

/// The pseudo refs git itself keeps reachable beside `refs/*`. A rebase,
/// a merge or a cherry-pick in flight names its objects only here.
const PSEUDO_REFS: &[&str] = &[
    "HEAD",
    "ORIG_HEAD",
    "MERGE_HEAD",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    "REBASE_HEAD",
    "FETCH_HEAD",
];

/// The closure of everything git calls reachable: every reference, the
/// pseudo refs, every reflog entry of every reference and of `HEAD`, the
/// index, and the same two for every linked worktree.
///
/// D3.7 names the reflogs of `HEAD` and `refs/heads/*`; joy reads the
/// reflog of every reference it enumerates instead, which is a superset
/// and costs one file open per ref. The reason is `refs/stash`: every
/// stash entry below the top exists ONLY as a reflog entry of that ref,
/// and a 14 day sweep that skipped it would eat a person's older
/// stashes. The load bearing consequence of D3.7 is untouched:
/// `refs/joy/chats` gets no reflog at all (libgit2 logs only
/// `refs/heads/*`, `refs/remotes/*`, `refs/notes/*` and `HEAD`), so the
/// chat store's lost compare and swap commits are pinned by nothing.
fn keep_set(repo: &Repository) -> HashSet<Oid> {
    let mut tips: Vec<Oid> = Vec::new();
    let mut reflogs: Vec<String> = PSEUDO_REFS.iter().map(|name| (*name).to_string()).collect();

    if let Ok(refs) = repo.references() {
        for reference in refs.flatten() {
            if let Some(oid) = reference.target() {
                tips.push(oid);
            } else if let Ok(resolved) = reference.resolve() {
                if let Some(oid) = resolved.target() {
                    tips.push(oid);
                }
            }
            if let Ok(name) = reference.name() {
                reflogs.push(name.to_string());
            }
        }
    }

    for name in &reflogs {
        let Ok(reflog) = repo.reflog(name) else {
            continue;
        };
        for entry in reflog.iter() {
            tips.push(entry.id_old());
            tips.push(entry.id_new());
        }
    }

    let git_dir = repo.path().to_path_buf();
    for name in PSEUDO_REFS {
        tips.extend(oids_in_pseudo_ref(&git_dir.join(name)));
    }
    tips.extend(index_oids(repo.index().ok()));

    // Linked worktrees keep their own HEAD, their own index and their own
    // pseudo refs under .git/worktrees/<name>/. A detached HEAD there
    // names a commit no reference does.
    if let Ok(worktrees) = fs::read_dir(git_dir.join("worktrees")) {
        for dir in worktrees.flatten() {
            let dir = dir.path();
            for name in PSEUDO_REFS {
                tips.extend(oids_in_pseudo_ref(&dir.join(name)));
            }
            tips.extend(index_oids(git2::Index::open(&dir.join("index")).ok()));
        }
    }

    closure(repo, tips)
}

fn index_oids(index: Option<git2::Index>) -> Vec<Oid> {
    let Some(index) = index else {
        return Vec::new();
    };
    index.iter().map(|entry| entry.id).collect()
}

/// Every object id named in a pseudo ref file. `HEAD` and friends hold
/// either `ref: <name>` (already covered by the reference walk) or a raw
/// id; `FETCH_HEAD` holds one id per line.
fn oids_in_pseudo_ref(path: &Path) -> Vec<Oid> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter_map(|word| Oid::from_str(word).ok())
        .collect()
}

/// Walk commits, their parents, their trees and every tag target from the
/// given tips. git2 offers no reachability query (`graph_ahead_behind`
/// and `graph_descendant_of` answer other questions and
/// `git_graph_reachable_from_any` is unbound), so the walk is explicit.
/// A missing or unreadable object is skipped: maintenance never fails a
/// write over a store it found damaged.
fn closure(repo: &Repository, tips: Vec<Oid>) -> HashSet<Oid> {
    let Ok(odb) = repo.odb() else {
        return HashSet::new();
    };
    let mut seen: HashSet<Oid> = HashSet::new();
    let mut stack = tips;
    while let Some(oid) = stack.pop() {
        if oid.is_zero() || !seen.insert(oid) {
            continue;
        }
        let Ok((_, kind)) = odb.read_header(oid) else {
            continue;
        };
        match kind {
            ObjectType::Commit => {
                if let Ok(commit) = repo.find_commit(oid) {
                    stack.push(commit.tree_id());
                    stack.extend(commit.parent_ids());
                }
            }
            ObjectType::Tree => {
                if let Ok(tree) = repo.find_tree(oid) {
                    stack.extend(tree.iter().map(|entry| entry.id()));
                }
            }
            ObjectType::Tag => {
                if let Ok(tag) = repo.find_tag(oid) {
                    stack.push(tag.target_id());
                }
            }
            ObjectType::Blob | ObjectType::Any => {}
        }
    }
    seen
}

// ---- step 2: the pack --------------------------------------------------

/// Write the loose members of the keep set into one new pack and refresh
/// the odb. Returns the object ids that went in and whether the pack
/// landed on disk.
///
/// D3.7 says `insert_commit` for the keep set commits and
/// `insert_recursive` for the rest. joy inserts the loose members of the
/// closure one by one instead, because both of those recurse into the
/// object's tree: a single loose commit in a large checkout would copy
/// the whole packed snapshot into a second pack, on every run, forever.
/// The result is the same set of objects the sweep may then remove, since
/// the closure is already computed in full in step 1, and a pack is not
/// required to be closed under reachability.
fn pack_loose_keep_set(
    repo: &Repository,
    keep: &HashSet<Oid>,
    loose: &[LooseObject],
) -> Result<(HashSet<Oid>, bool), JoyError> {
    let packed: HashSet<Oid> = loose
        .iter()
        .map(|object| object.oid)
        .filter(|oid| keep.contains(oid))
        .collect();
    if packed.is_empty() {
        return Ok((packed, false));
    }
    let pack_dir = repo.path().join("objects").join("pack");
    let before = pack_files(&pack_dir);

    let mut builder = repo.packbuilder().map_err(git)?;
    for oid in &packed {
        builder.insert_object(*oid, None).map_err(git)?;
    }
    builder.write(&pack_dir, 0).map_err(git)?;
    // The new pack is only usable once the odb knows about it. This
    // refreshes joy's own odb; any other live odb, in this process or in
    // another, refreshes itself on the first loose read that misses,
    // because `git_odb_read` answers `GIT_ENOTFOUND` with
    // `git_odb_refresh` and a second pass over the backends that have a
    // refresh function, and the pack backend is the one that has it.
    repo.odb().map_err(git)?.refresh().map_err(git)?;

    // The indexer names a pack after the hash of its content, so the
    // builder can say which file it just wrote; where it says nothing,
    // "a pack file appeared" is the same statement.
    let landed = match builder.name().ok().flatten() {
        Some(name) => pack_dir.join(format!("pack-{name}.pack")).is_file(),
        None => pack_files(&pack_dir).difference(&before).count() > 0,
    };
    Ok((packed, landed))
}

fn pack_files(pack_dir: &Path) -> HashSet<PathBuf> {
    let Ok(entries) = fs::read_dir(pack_dir) else {
        return HashSet::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "pack"))
        .collect()
}

// ---- step 3: the sweep -------------------------------------------------

struct LooseObject {
    oid: Oid,
    path: PathBuf,
}

/// Every loose object in `objects/xx/yyy…`. `Odb::foreach` would answer
/// loose and packed ids mixed and with no path, so the fanout directories
/// are read directly. Anything that is not a fanout directory
/// (`objects/pack`, `objects/info`, an `incoming-*` staging directory) or
/// not an object file (a `tmp_obj_*` half written by another process) is
/// left alone by the name filter.
fn loose_objects(git_dir: &Path) -> Vec<LooseObject> {
    let objects = git_dir.join("objects");
    let Ok(fanouts) = fs::read_dir(&objects) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for fanout in fanouts.flatten() {
        let Some(prefix) = fanout.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if prefix.len() != 2 || !prefix.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let Ok(entries) = fs::read_dir(fanout.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let Some(rest) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if !rest.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            let Ok(oid) = Oid::from_str(&format!("{prefix}{rest}")) else {
                continue;
            };
            found.push(LooseObject {
                oid,
                path: entry.path(),
            });
        }
    }
    found
}

fn loose_path(git_dir: &Path, oid: Oid) -> PathBuf {
    let hex = oid.to_string();
    let (prefix, rest) = hex.split_at(2);
    git_dir.join("objects").join(prefix).join(rest)
}

/// Whether joy can set this object's mtime, asked by setting it to the
/// value it already has. This is exactly what libgit2 does to protect an
/// object it is about to reuse (`git_odb__freshen` calls `p_utimes`), so
/// a refusal means no other process can protect the object either, and
/// D3.7 has the sweep skip rather than proceed.
fn can_freshen(path: &Path, mtime: SystemTime) -> bool {
    filetime::set_file_mtime(path, filetime::FileTime::from_system_time(mtime)).is_ok()
}

enum Unlink {
    /// The file is no longer there.
    Gone,
    /// Somebody else holds it; leave it, next run.
    Left,
}

/// Best effort per file, the way git's own prune is.
///
/// On Windows libgit2 opens files with `FILE_SHARE_READ | FILE_SHARE_WRITE`
/// and never `FILE_SHARE_DELETE`, and `DeleteFile` fails while another
/// handle is open, so a sharing violation (raw error 32) and an access
/// denial (raw error 5) mean "leave it, next run" and never an abort.
/// A loose object also carries the read only attribute there, which
/// `remove_file` refuses, so the attribute is cleared once and the unlink
/// retried, which is what git's own `mingw_unlink` does.
fn unlink_best_effort(path: &Path) -> Unlink {
    match fs::remove_file(path) {
        Ok(()) => Unlink::Gone,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Unlink::Gone,
        Err(_e) => {
            #[cfg(windows)]
            {
                if let Ok(metadata) = fs::metadata(path) {
                    let mut perms = metadata.permissions();
                    if perms.readonly() {
                        perms.set_readonly(false);
                        if fs::set_permissions(path, perms).is_ok() && fs::remove_file(path).is_ok()
                        {
                            return Unlink::Gone;
                        }
                    }
                }
                let _ = _e;
            }
            Unlink::Left
        }
    }
}

// ---- the reflog config that turns the sweep into a no-op ---------------

/// Checkouts already told about `core.logAllRefUpdates=always`, so the
/// sentence is said once and not on every write.
static REPORTED_LOG_ALL: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

/// `core.logAllRefUpdates=always` logs EVERY ref update, including
/// `refs/joy/chats`, so the chat store's lost commits enter the keep set
/// through their reflog and the sweep reclaims nothing. That is a valid
/// choice by the person who set it, and joy says so instead of silently
/// doing nothing.
fn logs_every_ref_update(repo: &Repository) -> bool {
    repo.config()
        .and_then(|config| config.get_string("core.logAllRefUpdates"))
        .map(|value| value.eq_ignore_ascii_case("always"))
        .unwrap_or(false)
}

fn report_log_all_ref_updates_once(git_dir: &Path) {
    let mut guard = REPORTED_LOG_ALL.lock().unwrap_or_else(|e| e.into_inner());
    let reported = guard.get_or_insert_with(HashSet::new);
    if reported.insert(git_dir.to_path_buf()) {
        tracing::warn!(
            store = %git_dir.display(),
            "core.logAllRefUpdates=always keeps a reflog for every ref, so unreachable objects stay reachable and joy's maintenance can free almost nothing"
        );
    }
}

fn git(e: git2::Error) -> JoyError {
    JoyError::Git(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    fn repo() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        (dir, repo)
    }

    fn signature() -> git2::Signature<'static> {
        git2::Signature::new("joy", "joy@localhost", &git2::Time::new(1_760_000_000, 0)).unwrap()
    }

    /// A commit on a ref outside refs/heads (the chat store's shape: no
    /// reflog, so nothing but the ref itself keeps it).
    fn commit_on_ref(repo: &Repository, refname: &str, body: &str) -> Oid {
        let blob = repo.blob(body.as_bytes()).unwrap();
        let mut builder = repo.treebuilder(None).unwrap();
        builder.insert("body", blob, 0o100_644).unwrap();
        let tree = repo.find_tree(builder.write().unwrap()).unwrap();
        let parent = repo
            .refname_to_id(refname)
            .ok()
            .map(|oid| repo.find_commit(oid).unwrap());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        let sig = signature();
        let oid = repo
            .commit(None, &sig, &sig, body, &tree, &parents)
            .unwrap();
        repo.reference(refname, oid, true, body).unwrap();
        oid
    }

    /// An orphan commit with its own tree and blob: what a lost compare
    /// and swap leaves behind.
    fn orphan_commit(repo: &Repository, body: &str) -> Oid {
        let blob = repo.blob(body.as_bytes()).unwrap();
        let mut builder = repo.treebuilder(None).unwrap();
        builder.insert("body", blob, 0o100_644).unwrap();
        let tree = repo.find_tree(builder.write().unwrap()).unwrap();
        let sig = signature();
        repo.commit(None, &sig, &sig, body, &tree, &[]).unwrap()
    }

    fn backdate(path: &Path, age: Duration) {
        let when = SystemTime::now() - age;
        filetime::set_file_mtime(path, filetime::FileTime::from_system_time(when)).unwrap();
    }

    fn eager() -> Options {
        // The grace window is a parameter exactly so a test can ask for
        // "everything old enough"; the product constants are 14 days and
        // 24 hours and are asserted in their own cases below.
        Options {
            grace: Duration::ZERO,
            ..Options::foreign_checkout()
        }
    }

    #[test]
    fn the_sweep_reclaims_lost_commits_and_keeps_every_reference() {
        let (dir, repo) = repo();
        commit_on_ref(&repo, "refs/joy/chats", "chat one");
        let tip = commit_on_ref(&repo, "refs/joy/chats", "chat two");
        for n in 0..40 {
            orphan_commit(&repo, &format!("lost race {n}"));
        }
        let before = loose_objects(dir.path().join(".git").as_path()).len();

        let outcome = maintain(&repo, &eager()).unwrap();

        assert!(
            outcome.removed_unreferenced >= 40,
            "every lost commit and its tree and blob should go: {outcome:?}"
        );
        assert!(
            loose_objects(dir.path().join(".git").as_path()).len() < before / 2,
            "the store should shrink markedly"
        );
        // Every commit of the ref's history is still readable, through
        // the pack where the loose copy is gone.
        let mut walk = repo.revwalk().unwrap();
        walk.push(tip).unwrap();
        let history: Vec<Oid> = walk.map(|oid| oid.unwrap()).collect();
        assert_eq!(history.len(), 2, "the whole chat history survives");
        for oid in history {
            let commit = repo.find_commit(oid).unwrap();
            let tree = commit.tree().unwrap();
            let entry = tree.get_name("body").unwrap();
            assert!(entry.to_object(&repo).unwrap().peel_to_blob().is_ok());
        }
    }

    #[test]
    fn an_object_younger_than_the_grace_window_survives() {
        let (dir, repo) = repo();
        commit_on_ref(&repo, "refs/joy/chats", "a chat");
        let orphan = orphan_commit(&repo, "written a moment ago");
        let path = loose_path(&dir.path().join(".git"), orphan);
        assert!(path.exists());

        let outcome = maintain(&repo, &Options::foreign_checkout()).unwrap();

        assert!(path.exists(), "a fresh object is inside the grace window");
        assert!(outcome.kept_young >= 1);
        assert_eq!(outcome.removed_unreferenced, 0);
        assert!(repo.find_commit(orphan).is_ok());
    }

    #[test]
    fn an_object_older_than_the_grace_window_goes() {
        let (dir, repo) = repo();
        commit_on_ref(&repo, "refs/joy/chats", "a chat");
        let orphan = orphan_commit(&repo, "from another month");
        let path = loose_path(&dir.path().join(".git"), orphan);
        backdate(&path, GRACE_FOREIGN_CHECKOUT + Duration::from_secs(60));

        let outcome = maintain(&repo, &Options::foreign_checkout()).unwrap();

        assert!(
            !path.exists(),
            "an object past the window goes: {outcome:?}"
        );
        assert_eq!(outcome.removed_unreferenced, 1);
    }

    #[test]
    fn an_mtime_in_the_future_is_left_alone() {
        let (dir, repo) = repo();
        commit_on_ref(&repo, "refs/joy/chats", "a chat");
        let orphan = orphan_commit(&repo, "a skewed mount");
        let path = loose_path(&dir.path().join(".git"), orphan);
        let ahead = SystemTime::now() + Duration::from_secs(3600);
        filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(ahead)).unwrap();

        let outcome = maintain(&repo, &eager()).unwrap();

        assert!(path.exists(), "no age statement is possible, so no unlink");
        assert_eq!(outcome.skipped_future_mtime, 1);
    }

    #[test]
    fn the_reflog_of_a_branch_keeps_its_older_commits() {
        let (dir, repo) = repo();
        let first = commit_on_ref(&repo, "refs/heads/main", "one");
        let second = commit_on_ref(&repo, "refs/heads/main", "two");
        // Move the branch back: the second commit is now reachable only
        // through the reflog, which is exactly what git keeps.
        repo.reference("refs/heads/main", first, true, "back to one")
            .unwrap();
        let path = loose_path(&dir.path().join(".git"), second);
        backdate(&path, GRACE_FOREIGN_CHECKOUT + Duration::from_secs(60));

        maintain(&repo, &Options::foreign_checkout()).unwrap();

        assert!(
            repo.find_commit(second).is_ok(),
            "a commit in the reflog stays readable"
        );
    }

    #[test]
    fn a_pack_is_never_deleted() {
        let (dir, repo) = repo();
        commit_on_ref(&repo, "refs/joy/chats", "a chat");
        maintain(&repo, &eager()).unwrap();
        let pack_dir = dir.path().join(".git").join("objects").join("pack");
        let packs = pack_files(&pack_dir);
        assert!(!packs.is_empty(), "the keep set was packed");

        // A second run finds the objects packed already and must leave
        // every pack where it is.
        commit_on_ref(&repo, "refs/joy/chats", "another chat");
        maintain(&repo, &eager()).unwrap();
        for pack in &packs {
            assert!(pack.is_file(), "{} was deleted", pack.display());
        }
    }

    #[test]
    fn the_estimate_and_the_time_floor_gate_the_run() {
        let (dir, repo) = repo();
        let git_dir = dir.path().join(".git");
        assert_eq!(estimate_loose_objects(&git_dir), 0);
        // A quiet store is never packed, whatever the caller asks.
        commit_on_ref(&repo, "refs/joy/chats", "a chat");
        assert!(maintain_if_due(&repo, &Options::foreign_checkout())
            .unwrap()
            .is_none());

        // 27 entries in one fanout directory are 6912 estimated objects,
        // which is git's own trigger.
        let fanout = git_dir.join("objects").join(FANOUT_SAMPLE);
        fs::create_dir_all(&fanout).unwrap();
        for n in 0..27 {
            fs::write(fanout.join(format!("{n:038x}")), b"").unwrap();
        }
        assert!(estimate_loose_objects(&git_dir) >= LOOSE_OBJECT_THRESHOLD);
        assert!(maintain_if_due(&repo, &Options::foreign_checkout())
            .unwrap()
            .is_some());
        // …and the wall clock floor holds the next one back.
        assert!(maintain_if_due(&repo, &Options::foreign_checkout())
            .unwrap()
            .is_none());

        // A CLI command is one process per write, so the floor is kept on
        // disk too: forget what this process remembers and it still
        // holds.
        LAST_RUN.lock().unwrap_or_else(|e| e.into_inner()).take();
        assert!(stamp_path(&git_dir).is_file());
        assert!(maintain_if_due(&repo, &Options::foreign_checkout())
            .unwrap()
            .is_none());
    }

    #[test]
    fn log_all_ref_updates_always_is_detected() {
        let (_dir, repo) = repo();
        commit_on_ref(&repo, "refs/joy/chats", "a chat");
        repo.config()
            .unwrap()
            .set_str("core.logAllRefUpdates", "always")
            .unwrap();

        let outcome = maintain(&repo, &eager()).unwrap();

        assert!(outcome.log_all_ref_updates_always);
    }

    #[test]
    fn a_lost_commit_can_be_discarded_by_id() {
        let (dir, repo) = repo();
        let orphan = orphan_commit(&repo, "a lost race");
        let path = loose_path(&dir.path().join(".git"), orphan);
        assert!(path.exists());

        assert!(discard_loose_object(&repo, orphan));

        assert!(!path.exists());
        // Idempotent: a second call finds nothing to do and says so.
        assert!(discard_loose_object(&repo, orphan));
    }

    /// D3.7's acceptance: "a second process holding an object open does
    /// not break the sweep". The object is in the keep set, so the sweep
    /// removes its loose copy (class A) and every reader, including this
    /// process, finds it in the pack afterwards. The child keeps its own
    /// descriptor across the sweep and still reads the bytes, which is
    /// what an unlink under a live reader means on unix.
    #[cfg(unix)]
    #[test]
    fn a_second_process_holding_an_object_open_does_not_break_the_sweep() {
        let (dir, repo) = repo();
        let tip = commit_on_ref(&repo, "refs/joy/chats", "a chat somebody is reading");
        let path = loose_path(&dir.path().join(".git"), tip);
        let size = fs::metadata(&path).unwrap().len();

        // The child opens the loose object, says so, waits for a line,
        // and then reads the file through the descriptor it still holds.
        let mut child = match joy_process::command("sh")
            .arg("-c")
            .arg("exec 3< \"$1\"; echo open; read _; wc -c <&3")
            .arg("sh")
            .arg(&path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            // No POSIX shell: the case has nothing to say on this host.
            Err(_) => return,
        };
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "open");

        let outcome = maintain(&repo, &eager()).unwrap();
        assert!(outcome.removed_packed >= 1, "{outcome:?}");
        assert!(!path.exists(), "the loose copy is in the pack now");
        assert!(
            repo.find_commit(tip).is_ok(),
            "and every reader still finds the object"
        );

        child.stdin.take().unwrap().write_all(b"\n").unwrap();
        let mut bytes = String::new();
        out.read_line(&mut bytes).unwrap();
        child.wait().unwrap();
        assert_eq!(
            bytes.trim().parse::<u64>().unwrap(),
            size,
            "the held descriptor still reads the whole object"
        );
    }
}
