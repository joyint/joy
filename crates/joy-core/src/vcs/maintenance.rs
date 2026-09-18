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
//!   by 256, the way git does), a wall clock floor per object store,
//!   and git's own threshold of 6700 loose objects. The run takes
//!   maintenance's own per store gate, and every path it touches comes
//!   off the repository's COMMON directory, because the linked
//!   worktrees of one checkout share one object store.
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

/// How long the keep set walk may take before the whole run is
/// abandoned. The walk is synchronous on a chat write and its cost is
/// the reachable object count: measured at about 15 us per object (14 402
/// reachable objects in 214 ms, release build), so a checkout with a
/// million objects would block a `joy chat send` for many seconds, and
/// the trigger fires on any store git itself would collect. One second
/// is the ceiling joy is willing to spend on somebody else's checkout.
pub const KEEP_SET_BUDGET: Duration = Duration::from_secs(1);

/// What a maintenance run is allowed to do.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// A loose object outside the keep set is removed only when it is at
    /// least this old (D3.7: 14 days in a checkout joy does not own,
    /// 24 hours in a store joy alone writes).
    pub grace: Duration,
    /// The estimated loose object count a run needs to see.
    pub loose_threshold: usize,
    /// The wall clock floor between two runs on one object store.
    pub min_interval: Duration,
    /// How long the keep set walk may take. A run that runs out of
    /// budget does NOTHING: a partial keep set would make the sweep
    /// delete reachable objects, so the only safe answer to a store too
    /// big to walk is to leave it alone (the store joy's own writes grow
    /// is small, and a checkout of that size has a git that maintains
    /// it).
    pub keep_set_budget: Duration,
}

impl Options {
    /// A checkout joy shares with a person and their own git: git's own
    /// 14 day grace window.
    pub fn foreign_checkout() -> Self {
        Self {
            grace: GRACE_FOREIGN_CHECKOUT,
            loose_threshold: LOOSE_OBJECT_THRESHOLD,
            min_interval: MIN_INTERVAL,
            keep_set_budget: KEEP_SET_BUDGET,
        }
    }

    /// A store joy alone writes (the platform's project clone, a desktop
    /// only store): 24 hours, which still covers a job container writing
    /// into the same object store.
    ///
    /// DEVIATION, reported at the package level rather than written
    /// into the design, which this package does not own: no caller
    /// selects this today, so a desktop only store and the platform's
    /// project clone both run on the 14 day window D3.7 gives a foreign
    /// checkout. The chat store's write path is the only caller there
    /// is and it cannot tell the two cases apart from where it stands:
    /// the same function serves a person's own checkout, where another
    /// git may be writing objects joy cannot see, and a store joy alone
    /// writes. Carrying the kind of store into that path is P8's change
    /// (the platform's project clone) and the desktop's own packaging;
    /// until then the only mistake this makes is the expensive one (a
    /// store swept 13 days later than it could be, which is the 38.89
    /// MiB of JOY-023C-1E held longer), never the unsafe one.
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
    /// Left for the next run: another process holds the file open. On
    /// Windows that is the raw OS error, 32 (ERROR_SHARING_VIOLATION) or
    /// 5 (ERROR_ACCESS_DENIED), read off the error and not guessed from
    /// the fact that an unlink failed; elsewhere it is `EBUSY`.
    pub skipped_in_use: usize,
    /// The unlink failed for a reason that is NOT another process
    /// holding the file. Counted apart from [`Outcome::skipped_in_use`]
    /// so that counter can answer the question it exists for.
    pub skipped_unlink_failed: usize,
    /// A keep set object the pack builder refused (a damaged loose
    /// object, or one another process removed between the walk and the
    /// pack). It is skipped and counted, never packed and above all
    /// never treated as class A, because its loose copy is the only
    /// copy there is.
    pub skipped_unpackable: usize,
    /// The keep set walk ran out of its budget, so this run did nothing
    /// at all: no pack and, above all, no sweep, because a partial keep
    /// set would remove reachable objects.
    pub abandoned_keep_set_budget: bool,
    /// `core.logAllRefUpdates=always` is set, so every ref update grows a
    /// reflog, the keep set grows with it and the sweep reclaims little
    /// or nothing. Reported once per checkout and process.
    pub log_all_ref_updates_always: bool,
}

/// Run maintenance when this object store is due for it: the loose
/// object estimate is over the threshold, the wall clock floor has
/// passed, and no other thread is already maintaining this store.
///
/// The gate is maintenance's OWN, not the per checkout gate of
/// [`crate::vcs::forge::checkout_gate`] that D3.7 first named. Two
/// reasons, both load bearing.
///
/// The first is that taking that gate here makes maintenance dead code
/// on the desktop. The chat transport locks the checkout gate and then
/// performs the whole chat write inside it, and that write is what calls
/// maintenance; a std `Mutex` is not reentrant, so a `try_lock` on the
/// same thread answers `WouldBlock` on EVERY desktop write, for ever,
/// and "the next write tries again" is false because the next write
/// holds the gate too. A blocking `lock` would deadlock instead.
///
/// The second is that maintenance does not need it. The checkout gate's
/// contract is "every path that MOVES refs on a checkout takes this gate
/// first" (JP-00DB-61), and maintenance moves no ref: it writes a pack
/// and unlinks loose objects. D3.7's argument for why that is safe
/// beside another writer (the keep set, the grace window, class A only
/// for objects that are in the pack joy just wrote) is an argument about
/// concurrent writers in general and holds for a writer in this process
/// exactly as it holds for one in another process.
///
/// What this gate gives is precise and smaller than "two runs never
/// overlap on one store": it is a `Mutex` in a process local map, so it
/// serialises the runs of THIS process. Two `joy chat send` processes
/// that both find the on disk stamp older than the floor can still run
/// on one store at the same time, because the stamp is read before the
/// gate is taken and neither is a lock. That overlap is benign rather
/// than merely tolerated: both walk the same refs to the same keep set,
/// class A only unlinks objects the running process has just written
/// into its own pack, class B only unlinks objects outside a keep set
/// both agree on, and no pack is ever removed. An advisory lock file
/// would make the statement absolute; it is not what this package
/// ships, and the design sentence that reads as if the gate gave that
/// is reported as a deviation.
///
/// Best effort: a store somebody else is maintaining right now is left
/// to them, and the next write tries again.
pub fn maintain_if_due(repo: &Repository, opts: &Options) -> Result<Option<Outcome>, JoyError> {
    let store = store_key(repo);
    if estimate_loose_objects(repo.commondir()) < opts.loose_threshold {
        return Ok(None);
    }
    if !due(&store, opts.min_interval) {
        return Ok(None);
    }
    let gate = maintenance_gate(&store);
    let _guard = match gate.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return Ok(None),
    };
    mark_run(&store);
    maintain(repo, opts).map(Some)
}

/// Pack and sweep this repository now, whatever the trigger says. The
/// caller owns the gate; [`maintain_if_due`] is the ordinary door.
pub fn maintain(repo: &Repository, opts: &Options) -> Result<Outcome, JoyError> {
    // The COMMON directory, never `repo.path()`: in a linked worktree
    // `repo.path()` is `.git/worktrees/<name>`, which holds no `objects`
    // directory at all. Addressed through it, the estimate answers 0
    // whatever the store holds, the sweep finds nothing, and
    // `discard_loose_object` reports a success it did not perform.
    let common = repo.commondir().to_path_buf();
    let mut outcome = Outcome {
        log_all_ref_updates_always: logs_every_ref_update(repo),
        ..Outcome::default()
    };
    if outcome.log_all_ref_updates_always {
        // Said to the person, not only to a log the CLI has no
        // subscriber for; the returned bool is for the test that pins
        // the "once" half.
        let _said = report_log_all_ref_updates_once(&common);
    }

    let Some(keep) = keep_set(repo, opts.keep_set_budget) else {
        // Out of budget. Nothing is packed and NOTHING is swept: a
        // partial keep set is worse than no maintenance, because the
        // sweep would read it as "these objects are unreachable".
        outcome.abandoned_keep_set_budget = true;
        return Ok(outcome);
    };
    outcome.keep_set = keep.len();
    let loose = loose_objects(&common);
    outcome.loose_seen = loose.len();

    let pack = pack_loose_keep_set(repo, &keep, &loose)?;
    outcome.packed = pack.oids.len();
    outcome.skipped_unpackable = pack.unpackable;

    let now = SystemTime::now();
    let mut probe = FreshenProbe::default();
    for object in &loose {
        // Class A: the object is in the pack joy just wrote, so every
        // reader finds it there. Age does not matter.
        if pack.landed && pack.oids.contains(&object.oid) {
            match unlink_best_effort(&object.path) {
                Unlink::Gone => outcome.removed_packed += 1,
                Unlink::InUse => outcome.skipped_in_use += 1,
                Unlink::Failed => outcome.skipped_unlink_failed += 1,
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
        if !probe.can_freshen(&object.path) {
            // libgit2 protects an object it is about to reuse with
            // `utimes`. Where that is refused, another process has no way
            // to protect this object, so joy does not remove it.
            outcome.skipped_unfreshenable += 1;
            continue;
        }
        match unlink_best_effort(&object.path) {
            Unlink::Gone => outcome.removed_unreferenced += 1,
            Unlink::InUse => outcome.skipped_in_use += 1,
            Unlink::Failed => outcome.skipped_unlink_failed += 1,
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
///
/// This is the one deletion in the package that bypasses the keep set
/// and the grace window, so the caller carries the whole burden of
/// proof: it may name only an object that THIS attempt created and that
/// the live history does not reach (see `chat_ref::commit_root`).
pub fn discard_loose_object(repo: &Repository, oid: Oid) -> bool {
    let path = loose_path(repo.commondir(), oid);
    if !path.exists() {
        return true;
    }
    matches!(unlink_best_effort(&path), Unlink::Gone)
}

/// git's cheap estimate: count one fanout directory and multiply by 256.
/// A fanout is one byte of a uniformly distributed hash, so one directory
/// carries a 256th of the store.
///
/// `store_dir` is the directory that HOLDS `objects`, which is the
/// repository's common directory: a linked worktree's own git directory
/// has no object store of its own.
pub fn estimate_loose_objects(store_dir: &Path) -> usize {
    let sample = store_dir.join("objects").join(FANOUT_SAMPLE);
    let Ok(entries) = fs::read_dir(&sample) else {
        return 0;
    };
    entries.filter(|e| e.is_ok()).count().saturating_mul(256)
}

// ---- the trigger -------------------------------------------------------

/// When each object store last ran maintenance in this process. Per
/// canonical store, never a process global counter: the old chat store
/// counter fired on the first write of every process, so a CLI run ran it
/// every time and a long lived desktop every 64 writes, neither of which
/// is a statement about the store.
static LAST_RUN: Mutex<Option<HashMap<PathBuf, Instant>>> = Mutex::new(None);

/// One maintenance gate per object store. Lazy, never dropped: a gate is
/// a few bytes and a process touches a handful of stores. Keyed exactly
/// like [`LAST_RUN`], so the floor and the gate can never disagree about
/// which store they are talking about.
static MAINTENANCE_GATES: Mutex<Option<HashMap<PathBuf, std::sync::Arc<Mutex<()>>>>> =
    Mutex::new(None);

fn maintenance_gate(store: &Path) -> std::sync::Arc<Mutex<()>> {
    MAINTENANCE_GATES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .entry(store.to_path_buf())
        .or_default()
        .clone()
}

/// The key one object store is known by, for the floor and for the gate:
/// the repository's common directory, canonicalised.
///
/// Canonical because D3.7 says per canonical checkout: two handles
/// reached through different paths (a symlinked home, a bind mount) are
/// one store and must not get two floors. The COMMON directory rather
/// than `repo.path()` because every linked worktree of a checkout shares
/// one object store, and the object store is the thing being maintained.
fn store_key(repo: &Repository) -> PathBuf {
    let common = repo.commondir();
    common
        .canonicalize()
        .unwrap_or_else(|_| common.to_path_buf())
}

/// The floor is per STORE, and a CLI command is one process per write, so
/// an in-process map alone would be no floor at all for the host that
/// needs it most: every `joy chat send` would pack again. The stamp is a
/// file joy writes inside the git directory and reads by its mtime;
/// where it cannot be written (a read only or foreign owned `.git`) the
/// in-process map still holds. It is the one piece of state joy leaves
/// in a checkout it does not own. D3.7 asks for a wall clock floor per
/// checkout and does not say where it lives; the file is this package's
/// addition and is reported as a deviation, because the design document
/// is the shared contract of J0..J11 and no package edits it.
fn stamp_path(store: &Path) -> PathBuf {
    store.join("joy").join("maintenance-stamp")
}

fn due(store: &Path, min_interval: Duration) -> bool {
    {
        let guard = LAST_RUN.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(last) = guard.as_ref().and_then(|m| m.get(store)) {
            if last.elapsed() < min_interval {
                return false;
            }
        }
    }
    let Ok(stamped) = fs::metadata(stamp_path(store)).and_then(|m| m.modified()) else {
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

fn mark_run(store: &Path) {
    {
        let mut guard = LAST_RUN.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .get_or_insert_with(HashMap::new)
            .insert(store.to_path_buf(), Instant::now());
    }
    let stamp = stamp_path(store);
    if let Some(parent) = stamp.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&stamp, b"");
    let _ = filetime::set_file_mtime(&stamp, filetime::FileTime::now());
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
/// `None` means the walk ran out of `budget` and the caller must do
/// nothing at all: an incomplete keep set is not a keep set.
fn keep_set(repo: &Repository, budget: Duration) -> Option<HashSet<Oid>> {
    let deadline = Instant::now().checked_add(budget);
    let mut tips: Vec<Oid> = Vec::new();
    let mut reflogs: Vec<String> = PSEUDO_REFS.iter().map(|name| (*name).to_string()).collect();

    if let Ok(refs) = repo.references() {
        for (n, reference) in refs.flatten().enumerate() {
            // The budget covers the PROLOGUE, not only the object walk.
            // Enumerating every reference and opening a reflog for each
            // of them is one file open per ref, so a checkout with tens
            // of thousands of refs (a large fork, a tag heavy release
            // repository) would spend a chat write's whole latency here
            // before the walk ever starts. One clock read per 256 refs,
            // the same rate the walk uses.
            if n.is_multiple_of(256) && over_budget(deadline) {
                return None;
            }
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

    for (n, name) in reflogs.iter().enumerate() {
        if n.is_multiple_of(256) && over_budget(deadline) {
            return None;
        }
        let Ok(reflog) = repo.reflog(name) else {
            continue;
        };
        for entry in reflog.iter() {
            tips.push(entry.id_old());
            tips.push(entry.id_new());
        }
    }

    // This worktree's own git directory (a linked worktree keeps its
    // HEAD, its index and its pseudo refs there) AND the common one (the
    // main worktree's). In an ordinary checkout the two are the same
    // path and the duplicate ids cost nothing: the closure is a set.
    let git_dir = repo.path().to_path_buf();
    let common = repo.commondir().to_path_buf();
    for name in PSEUDO_REFS {
        tips.extend(oids_in_pseudo_ref(&git_dir.join(name)));
        if common != git_dir {
            tips.extend(oids_in_pseudo_ref(&common.join(name)));
        }
    }
    tips.extend(index_oids(repo.index().ok()));
    if common != git_dir {
        tips.extend(index_oids(git2::Index::open(&common.join("index")).ok()));
    }

    // Linked worktrees keep their own HEAD, their own index and their own
    // pseudo refs under .git/worktrees/<name>/, which lives in the COMMON
    // directory. A detached HEAD there names a commit no reference does.
    if over_budget(deadline) {
        return None;
    }
    if let Ok(worktrees) = fs::read_dir(common.join("worktrees")) {
        for dir in worktrees.flatten() {
            if over_budget(deadline) {
                return None;
            }
            let dir = dir.path();
            for name in PSEUDO_REFS {
                tips.extend(oids_in_pseudo_ref(&dir.join(name)));
            }
            tips.extend(index_oids(git2::Index::open(&dir.join("index")).ok()));
        }
    }

    closure(repo, tips, deadline)
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

fn over_budget(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|end| Instant::now() >= end)
}

/// Walk commits, their parents, their trees and every tag target from the
/// given tips. git2 offers no reachability query (`graph_ahead_behind`
/// and `graph_descendant_of` answer other questions and
/// `git_graph_reachable_from_any` is unbound), so the walk is explicit.
/// A missing or unreadable object is skipped: maintenance never fails a
/// write over a store it found damaged.
///
/// The walk is bounded by `deadline`, because it is synchronous on a chat
/// write and its cost is the reachable object count (about 15 us per
/// object). Running out of time answers `None`, never a partial set: the
/// sweep reads "not in the keep set" as "unreachable", so half a keep set
/// deletes live objects. `None` for the deadline means the budget could
/// not be expressed as an instant, and then the walk simply runs.
fn closure(repo: &Repository, tips: Vec<Oid>, deadline: Option<Instant>) -> Option<HashSet<Oid>> {
    let Ok(odb) = repo.odb() else {
        return Some(HashSet::new());
    };
    let mut seen: HashSet<Oid> = HashSet::new();
    let mut stack = tips;
    let mut visited = 0usize;
    if over_budget(deadline) {
        // The tips alone took the budget, or the caller asked for none.
        return None;
    }
    while let Some(oid) = stack.pop() {
        // One clock read per 256 objects: the walk itself is file opens,
        // so the check has to cost nothing next to them.
        visited += 1;
        if visited.is_multiple_of(256) && over_budget(deadline) {
            return None;
        }
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
    Some(seen)
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
) -> Result<Packed, JoyError> {
    let candidates: Vec<Oid> = loose
        .iter()
        .map(|object| object.oid)
        .filter(|oid| keep.contains(oid))
        .collect();
    if candidates.is_empty() {
        return Ok(Packed::default());
    }
    // The common directory again: `.git/worktrees/<name>/objects/pack`
    // is a directory no odb ever reads.
    let pack_dir = repo.commondir().join("objects").join("pack");
    let before = pack_files(&pack_dir);

    let mut builder = repo.packbuilder().map_err(git)?;
    let mut packed: HashSet<Oid> = HashSet::new();
    let mut unpackable = 0usize;
    for oid in candidates {
        // One damaged loose object must not end maintenance for this
        // store for ever. The closure keeps an oid whose header it could
        // not read (it is in the keep set, so the sweep leaves it alone
        // either way), and the builder refuses exactly that object;
        // aborting here would make `maintain` answer `Err` on every run
        // while the caller swallows it, which is the module's own rule
        // ("maintenance never fails a write over a store it found
        // damaged") broken in the quietest possible way.
        if builder.insert_object(oid, None).is_ok() {
            packed.insert(oid);
        } else {
            unpackable += 1;
        }
    }
    if packed.is_empty() {
        // Nothing went in, so nothing may be treated as packed. There is
        // no pack to write either.
        return Ok(Packed {
            unpackable,
            ..Packed::default()
        });
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
    Ok(Packed {
        oids: packed,
        landed,
        unpackable,
    })
}

/// What step 2 produced: the objects that really went into the new pack
/// (never the ones that were merely offered to it), whether the pack
/// landed on disk, and how many keep set objects the builder refused.
#[derive(Default)]
struct Packed {
    oids: HashSet<Oid>,
    landed: bool,
    unpackable: usize,
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
fn loose_objects(store_dir: &Path) -> Vec<LooseObject> {
    let objects = store_dir.join("objects");
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
            // The LENGTH matters as much as the alphabet.
            // `Oid::from_str` accepts any prefix of 1 to 40 hex
            // characters and zero fills the rest, so a stray
            // `objects/ab/cd` would become the object id `abcd000...0`,
            // which is in no keep set and would be swept once it is old
            // enough. A sha1 object name is the other 38 digits of the
            // id; a store with another hash simply parses as no id at
            // all here and is left alone, which is the safe direction.
            if rest.len() != 38 || !rest.bytes().all(|b| b.is_ascii_hexdigit()) {
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

fn loose_path(store_dir: &Path, oid: Oid) -> PathBuf {
    let hex = oid.to_string();
    let (prefix, rest) = hex.split_at(2);
    store_dir.join("objects").join(prefix).join(rest)
}

/// Whether an object could be freshened, asked WITHOUT touching a single
/// object.
///
/// libgit2 protects an object it is about to reuse by setting its mtime
/// (`git_odb__freshen` calls `p_utimes`), and D3.7 has the sweep skip
/// where that call cannot succeed, because then no other process can
/// protect the object either. The obvious probe, writing the mtime joy
/// just read back onto the object, is the one thing this must NOT do: a
/// freshen by another process between the stat and the probe would be
/// erased by it, and joy would delete the object the erased freshen was
/// protecting, which is the exact concurrency argument the rule exists
/// to honour.
///
/// So the probe runs on a scratch file joy creates in the same fanout
/// directory and removes again. It answers the mount question (a read
/// only mount, a directory joy may not write, a filesystem that refuses
/// `utimes`), and the scratch file's own owner answers the ownership
/// question: a loose object owned by somebody else cannot be `utimes`ed
/// by joy even where the directory is writable. One probe per fanout
/// directory, so at most 256 per run.
///
/// The price of the ownership half is worth naming, because it is paid
/// on the store D5 is about. In the platform's job container the project
/// clone is bind mounted read write and an agent's own `git commit`
/// writes into the same object store, typically as another uid. Every
/// object it leaves there is `skipped_unfreshenable` for ever: class B
/// never collects it, so that store grows until the maintenance owner
/// D5 gives it (the platform's sync worker lane, which does not exist
/// yet) sweeps it as the uid that owns the objects. The counter says so,
/// and today nobody reads the counter: the chat store drops the whole
/// `Outcome`. This is faithful to the rule (without a working `utimes`
/// another process cannot protect the object, so joy must not remove
/// it) and it is not free.
#[derive(Default)]
struct FreshenProbe {
    dirs: HashMap<PathBuf, Option<Owner>>,
}

impl FreshenProbe {
    fn can_freshen(&mut self, object: &Path) -> bool {
        let Some(dir) = object.parent() else {
            return false;
        };
        let probed = match self.dirs.get(dir) {
            Some(probed) => *probed,
            None => {
                let probed = probe_directory(dir);
                self.dirs.insert(dir.to_path_buf(), probed);
                probed
            }
        };
        let Some(owner) = probed else {
            return false;
        };
        same_owner(object, owner)
    }
}

/// The owner a `utimes` needs to match. On unix the effective uid, read
/// off a file joy just created rather than asked of libc; elsewhere
/// there is no cheap equivalent and the directory probe stands alone.
#[cfg(unix)]
type Owner = u32;
#[cfg(not(unix))]
type Owner = ();

/// Create one scratch file, set its mtime, read its owner back, remove
/// it. The name is not 38 hex digits, so neither joy's own sweep nor git
/// mistakes it for an object.
fn probe_directory(dir: &Path) -> Option<Owner> {
    let scratch = dir.join(format!("tmp_joy_freshen_probe_{}", std::process::id()));
    if fs::write(&scratch, b"").is_err() {
        return None;
    }
    let settable = filetime::set_file_mtime(&scratch, filetime::FileTime::now()).is_ok();
    let owner = owner_of(&scratch);
    let _ = fs::remove_file(&scratch);
    if settable {
        owner
    } else {
        None
    }
}

#[cfg(unix)]
fn owner_of(path: &Path) -> Option<Owner> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).ok().map(|m| m.uid())
}

#[cfg(not(unix))]
fn owner_of(_path: &Path) -> Option<Owner> {
    Some(())
}

#[cfg(unix)]
fn same_owner(path: &Path, owner: Owner) -> bool {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path)
        .map(|m| m.uid() == owner)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn same_owner(_path: &Path, _owner: Owner) -> bool {
    true
}

enum Unlink {
    /// The file is no longer there.
    Gone,
    /// Another process holds it open; leave it, next run.
    InUse,
    /// The unlink failed for another reason, which is not the same
    /// statement and is not counted as if it were.
    Failed,
}

/// ERROR_SHARING_VIOLATION: another handle is open on the file.
#[cfg(windows)]
const ERROR_SHARING_VIOLATION: i32 = 32;
/// ERROR_ACCESS_DENIED, which a delete under an open handle also answers.
#[cfg(windows)]
const ERROR_ACCESS_DENIED: i32 = 5;
/// git's own `mingw_unlink` waits 0, 1, 10, 20 and 40 ms between tries.
#[cfg(windows)]
const UNLINK_RETRY_MS: &[u64] = &[0, 1, 10, 20, 40];

/// Best effort per file, the way git's own prune is.
///
/// On Windows libgit2 opens files with `FILE_SHARE_READ | FILE_SHARE_WRITE`
/// and never `FILE_SHARE_DELETE`, and `DeleteFile` fails while another
/// handle is open, so a sharing violation (raw error 32) and an access
/// denial (raw error 5) mean "leave it, next run" and never an abort. The
/// raw error is read, not inferred: `skipped_in_use` has to mean what its
/// name says or the counter answers no question. A loose object also
/// carries the read only attribute there, which `remove_file` refuses, so
/// the attribute is cleared and the unlink retried on git's own ladder;
/// when the retries run out the attribute goes back on, because an object
/// joy has decided to KEEP must not be left with its protection removed.
#[cfg(windows)]
fn unlink_best_effort(path: &Path) -> Unlink {
    let mut cleared_readonly = false;
    let mut last_error = None;
    for wait in UNLINK_RETRY_MS {
        if *wait > 0 {
            std::thread::sleep(Duration::from_millis(*wait));
        }
        match fs::remove_file(path) {
            Ok(()) => return Unlink::Gone,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Unlink::Gone,
            Err(e) => {
                last_error = e.raw_os_error();
                if !cleared_readonly {
                    cleared_readonly = clear_readonly(path);
                }
            }
        }
    }
    if cleared_readonly {
        restore_readonly(path);
    }
    match last_error {
        Some(ERROR_SHARING_VIOLATION) | Some(ERROR_ACCESS_DENIED) => Unlink::InUse,
        _ => Unlink::Failed,
    }
}

/// `EBUSY`: the one "somebody else holds this" answer a unix unlink has
/// (a file that is a mount point, a swap file, a running text image).
#[cfg(not(windows))]
const EBUSY: i32 = 16;

#[cfg(not(windows))]
fn unlink_best_effort(path: &Path) -> Unlink {
    match fs::remove_file(path) {
        Ok(()) => Unlink::Gone,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Unlink::Gone,
        Err(e) if e.raw_os_error() == Some(EBUSY) => Unlink::InUse,
        Err(_) => Unlink::Failed,
    }
}

// The lint warns that clearing the read only bit makes a file world
// writable ON UNIX. This is the windows branch, where the bit is
// `FILE_ATTRIBUTE_READONLY` and clearing it is the only way to unlink
// the object git wrote.
#[allow(clippy::permissions_set_readonly_false)]
#[cfg(windows)]
fn clear_readonly(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    let mut perms = metadata.permissions();
    if !perms.readonly() {
        return false;
    }
    perms.set_readonly(false);
    fs::set_permissions(path, perms).is_ok()
}

#[cfg(windows)]
fn restore_readonly(path: &Path) {
    if let Ok(metadata) = fs::metadata(path) {
        let mut perms = metadata.permissions();
        perms.set_readonly(true);
        let _ = fs::set_permissions(path, perms);
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

/// The sentence itself, in one place so a test can read it.
fn log_all_ref_updates_sentence(store: &Path) -> String {
    format!(
        "Warning: core.logAllRefUpdates=always is set in {}, so git keeps a reflog for every ref. \
Unreachable objects stay reachable through it and joy's maintenance can free almost nothing. \
Unset it, or set it to true, to let joy reclaim the store.",
        native(store)
    )
}

/// A path in the spelling the machine writes it in.
///
/// libgit2 answers every path with forward slashes, on Windows too
/// (`git_repository_commondir` and everything built from it), so a
/// sentence that printed it raw told a Windows person to look in
/// `C:/Users/.../.git/`, which is not what anything else on that
/// machine calls it (JOY-02A7-A2 finding 7).
fn native(path: &Path) -> String {
    let shown = path.display().to_string();
    #[cfg(windows)]
    let shown = shown.replace('/', "\\");
    shown
}

/// Say it once per store and process. Answers whether THIS call said it.
///
/// On stderr, not only as a `tracing` event. The host D3.7 singles out
/// here is the CLI ("a CLI command is one process per write"), and the
/// `joy` binary installs no tracing subscriber at all, so an event alone
/// is dropped on the floor exactly where the sentence is needed: the
/// keep set grows to cover every lost chat commit, the sweep reclaims
/// nothing on every run for ever, and the person is told nothing. The
/// desktop and the platform do install a subscriber and get the
/// structured event as well.
fn report_log_all_ref_updates_once(git_dir: &Path) -> bool {
    {
        let mut guard = REPORTED_LOG_ALL.lock().unwrap_or_else(|e| e.into_inner());
        if !guard
            .get_or_insert_with(HashSet::new)
            .insert(git_dir.to_path_buf())
        {
            return false;
        }
    }
    eprintln!("{}", log_all_ref_updates_sentence(git_dir));
    tracing::warn!(
        store = %native(git_dir),
        "core.logAllRefUpdates=always keeps a reflog for every ref, so unreachable objects stay reachable and joy's maintenance can free almost nothing"
    );
    true
}

fn git(e: git2::Error) -> JoyError {
    JoyError::Git(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    // The one case that needs them holds an object open from a second
    // process, which takes a POSIX shell: unix only, and so are they.
    #[cfg(unix)]
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
        set_mtime(path, SystemTime::now() - age);
    }

    /// An object's mtime, set the way a case needs it.
    ///
    /// git writes a loose object read only (mode 0444, which is
    /// `FILE_ATTRIBUTE_READONLY` on Windows), and setting a time on it
    /// opens it for write, which Windows refuses: every case that ages
    /// an object failed there with "Access is denied" (JOY-02A7-A2
    /// finding 7). The attribute is cleared for the one call and put
    /// back, so the case still runs against the store git would have
    /// left behind.
    #[allow(clippy::permissions_set_readonly_false)]
    fn set_mtime(path: &Path, when: SystemTime) {
        #[cfg(windows)]
        let was_readonly = {
            let readonly = fs::metadata(path).unwrap().permissions().readonly();
            if readonly {
                let mut perms = fs::metadata(path).unwrap().permissions();
                perms.set_readonly(false);
                fs::set_permissions(path, perms).unwrap();
            }
            readonly
        };
        filetime::set_file_mtime(path, filetime::FileTime::from_system_time(when)).unwrap();
        #[cfg(windows)]
        if was_readonly {
            let mut perms = fs::metadata(path).unwrap().permissions();
            perms.set_readonly(true);
            fs::set_permissions(path, perms).unwrap();
        }
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
        set_mtime(&path, ahead);

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
        // disk too: forget what this process remembers about THIS store
        // (and only this one, the map belongs to the whole test binary)
        // and the floor still holds.
        LAST_RUN
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
            .unwrap()
            .remove(&store_key(&repo));
        assert!(stamp_path(&store_key(&repo)).is_file());
        assert!(maintain_if_due(&repo, &Options::foreign_checkout())
            .unwrap()
            .is_none());
    }

    #[test]
    fn log_all_ref_updates_always_is_detected_and_said_once() {
        let (dir, repo) = repo();
        commit_on_ref(&repo, "refs/joy/chats", "a chat");
        repo.config()
            .unwrap()
            .set_str("core.logAllRefUpdates", "always")
            .unwrap();

        let outcome = maintain(&repo, &eager()).unwrap();

        assert!(outcome.log_all_ref_updates_always);
        // …and the run SAID so. Asserting the bool on the struct alone
        // would pass just as well with the sentence going nowhere, which
        // is what it did: the CLI installs no tracing subscriber, so the
        // warning event was dropped on the host D3.7 singles out. The
        // run above is the first report for this store, so a second ask
        // must answer false.
        let store = repo.commondir().to_path_buf();
        assert!(
            !report_log_all_ref_updates_once(&store),
            "the run must have reported it already, and only once"
        );
        let sentence = log_all_ref_updates_sentence(&store);
        assert!(sentence.contains("core.logAllRefUpdates=always"));
        // The path the machine writes, not the one libgit2 answers:
        // `commondir` is all forward slashes, on Windows too. The
        // directory's own name is what is compared, because on the
        // Windows runner the temp root is spelled RUNNER~1 by the
        // environment and runneradmin by libgit2, and both are right.
        let name = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(sentence.contains(&name), "{sentence}");
        assert!(
            !sentence.contains('/') || cfg!(not(windows)),
            "the machine's own separators are expected: {sentence}"
        );
    }

    /// The first ask for a store reports, every later one is silent.
    #[test]
    fn the_log_all_sentence_is_said_once_per_store() {
        let store = tempfile::tempdir().unwrap();
        assert!(report_log_all_ref_updates_once(store.path()));
        assert!(!report_log_all_ref_updates_once(store.path()));
    }

    /// Both windows D3.7 names, pinned. `owned_store` has no caller yet
    /// (a deviation reported at the package level, not written into the
    /// design, which this package does not own), so the constant would
    /// otherwise be free to rot unnoticed.
    #[test]
    fn the_two_grace_windows_are_the_ones_the_design_names() {
        assert_eq!(
            Options::foreign_checkout().grace,
            Duration::from_secs(14 * 24 * 60 * 60)
        );
        assert_eq!(
            Options::owned_store().grace,
            Duration::from_secs(24 * 60 * 60)
        );
        assert_eq!(
            Options::owned_store().loose_threshold,
            Options::foreign_checkout().loose_threshold
        );
    }

    /// One damaged loose object must not end maintenance for this store
    /// for ever. The closure keeps an oid whose header it cannot read,
    /// the pack builder refuses exactly that object, and aborting there
    /// would make every later run answer `Err` into a caller that
    /// swallows it: no pack and no sweep again, silently.
    #[test]
    fn a_damaged_object_is_skipped_instead_of_ending_maintenance() {
        let (dir, repo) = repo();
        let blob = repo.blob(b"the object that goes bad").unwrap();
        let mut builder = repo.treebuilder(None).unwrap();
        builder.insert("body", blob, 0o100_644).unwrap();
        let tree = repo.find_tree(builder.write().unwrap()).unwrap();
        let sig = signature();
        let commit = repo.commit(None, &sig, &sig, "a chat", &tree, &[]).unwrap();
        repo.reference("refs/joy/chats", commit, true, "a chat")
            .unwrap();
        // Damage the blob in place: the name is still a valid object id,
        // so it is in the keep set through the tree, and nothing can
        // read it.
        // A loose object file is created read only, so it is replaced
        // rather than written over.
        let damaged = loose_path(&dir.path().join(".git"), blob);
        fs::remove_file(&damaged).unwrap();
        fs::write(&damaged, b"not zlib, not an object").unwrap();

        let outcome = maintain(&repo, &eager()).unwrap();

        assert!(outcome.skipped_unpackable >= 1, "{outcome:?}");
        assert!(damaged.exists(), "a keep set object is never swept");
        // and the run is repeatable, which is the whole point
        let second = maintain(&repo, &eager()).unwrap();
        assert!(second.skipped_unpackable >= 1, "{second:?}");
    }

    /// `Oid::from_str` accepts any prefix of 1 to 40 hex characters and
    /// zero fills the rest, so a short all hex file name would become an
    /// object id nothing keeps, and the sweep would unlink a file that
    /// is not an object at all.
    #[test]
    fn a_short_hex_file_name_is_not_an_object() {
        let (dir, repo) = repo();
        commit_on_ref(&repo, "refs/joy/chats", "a chat");
        let stray = dir.path().join(".git").join("objects").join("ab");
        fs::create_dir_all(&stray).unwrap();
        let stray = stray.join("cd");
        fs::write(&stray, b"somebody left this here").unwrap();
        backdate(&stray, GRACE_FOREIGN_CHECKOUT + Duration::from_secs(60));

        let found = loose_objects(&dir.path().join(".git"));
        assert!(
            !found.iter().any(|object| object.path == stray),
            "a name shorter than 38 hex digits is not an object name"
        );
        maintain(&repo, &eager()).unwrap();
        assert!(stray.exists(), "and the sweep leaves it where it is");
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
        // A host without a POSIX shell cannot be asked this question at
        // all, so the case says so and stops instead of failing: a panic
        // there would rename "not testable here" to "broken".
        let spawned = joy_process::command("sh")
            .arg("-c")
            .arg("exec 3< \"$1\"; echo open; read _; wc -c <&3")
            .arg("sh")
            .arg(&path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn();
        let Ok(mut child) = spawned else {
            eprintln!("skipped: no POSIX shell on PATH to hold an object open");
            return;
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
    /// The bug that made maintenance dead code on the desktop: the chat
    /// transport holds the per checkout gate across the whole write, and
    /// the write is what calls maintenance. A `try_lock` on that gate
    /// answers `WouldBlock` on the same thread, for ever.
    #[test]
    fn maintenance_runs_while_the_write_path_holds_the_checkout_gate() {
        let (dir, repo) = repo();
        commit_on_ref(&repo, "refs/joy/chats", "a chat");
        let fanout = dir.path().join(".git").join("objects").join(FANOUT_SAMPLE);
        fs::create_dir_all(&fanout).unwrap();
        for n in 0..27 {
            fs::write(fanout.join(format!("{n:038x}")), b"").unwrap();
        }

        let gate = crate::vcs::forge::checkout_gate(dir.path());
        let _held = gate.lock().unwrap_or_else(|e| e.into_inner());

        assert!(
            maintain_if_due(&repo, &Options::foreign_checkout())
                .unwrap()
                .is_some(),
            "the write path's own gate must not silence maintenance"
        );
    }

    /// In a linked worktree `repo.path()` is `.git/worktrees/<name>`,
    /// which has no object store at all: every path has to come off the
    /// common directory or the run is a silent no-op that reports
    /// success.
    #[test]
    fn a_linked_worktree_maintains_the_shared_object_store() {
        let (dir, repo) = repo();
        let tip = commit_on_ref(&repo, "refs/heads/main", "one");
        repo.set_head("refs/heads/main").unwrap();
        let _ = tip;
        // A second tempdir, never a fixed name beside the first one: a
        // path like `/tmp/linked-worktree` is shared by every concurrent
        // run of this binary (two worktrees, a CI matrix) and a failing
        // run would leave it behind and poison the next one.
        let outside = tempfile::tempdir().unwrap();
        let wt_dir = outside.path().join("linked-worktree");
        let worktree = repo.worktree("linked", &wt_dir, None).unwrap();
        let wrepo = Repository::open_from_worktree(&worktree).unwrap();

        // the store is the main repository's, whichever handle asks
        let common = dir.path().join(".git");
        assert_ne!(wrepo.path(), common.as_path());
        let orphan = orphan_commit(&repo, "a lost race");
        let path = loose_path(&common, orphan);
        backdate(&path, GRACE_FOREIGN_CHECKOUT + Duration::from_secs(60));
        let fanout = common.join("objects").join(FANOUT_SAMPLE);
        fs::create_dir_all(&fanout).unwrap();
        for n in 0..27 {
            fs::write(fanout.join(format!("{n:038x}")), b"").unwrap();
        }

        assert!(estimate_loose_objects(wrepo.commondir()) >= LOOSE_OBJECT_THRESHOLD);
        let outcome = maintain_if_due(&wrepo, &Options::foreign_checkout())
            .unwrap()
            .expect("a linked worktree sees the store it shares");

        assert!(outcome.loose_seen > 0, "{outcome:?}");
        assert!(!path.exists(), "the sweep reaches the shared store");
        // …and a discard through the worktree handle removes the real
        // file instead of reporting a success it did not perform.
        let second = orphan_commit(&repo, "another lost race");
        let second_path = loose_path(&common, second);
        assert!(second_path.exists());
        assert!(discard_loose_object(&wrepo, second));
        assert!(!second_path.exists());
    }

    /// The keep set walk is synchronous on a chat write, so it is
    /// bounded. Running out of budget must abandon the whole run: a
    /// partial keep set would make the sweep delete live objects.
    #[test]
    fn a_keep_set_over_budget_abandons_the_run_instead_of_sweeping_blind() {
        let (dir, repo) = repo();
        let tip = commit_on_ref(&repo, "refs/joy/chats", "a chat");
        let orphan = orphan_commit(&repo, "old enough to go");
        let path = loose_path(&dir.path().join(".git"), orphan);
        backdate(&path, GRACE_FOREIGN_CHECKOUT + Duration::from_secs(60));

        let outcome = maintain(
            &repo,
            &Options {
                keep_set_budget: Duration::ZERO,
                ..eager()
            },
        )
        .unwrap();

        assert!(outcome.abandoned_keep_set_budget, "{outcome:?}");
        assert_eq!(outcome.removed_unreferenced, 0);
        assert_eq!(outcome.removed_packed, 0);
        assert_eq!(outcome.packed, 0);
        assert!(path.exists(), "nothing is swept without a full keep set");
        assert!(repo.find_commit(tip).is_ok());
    }

    /// D3.7's concurrency argument rests on another process being able to
    /// protect an object with `utimes`. The freshen probe must therefore
    /// never write on an object itself: a freshen that landed between
    /// joy's stat and joy's probe would be erased by it.
    #[test]
    fn the_freshen_probe_leaves_every_object_mtime_where_it_was() {
        let (dir, repo) = repo();
        commit_on_ref(&repo, "refs/joy/chats", "a chat");
        let orphan = orphan_commit(&repo, "inside the window");
        let path = loose_path(&dir.path().join(".git"), orphan);
        let before = fs::metadata(&path).unwrap().modified().unwrap();

        maintain(&repo, &Options::foreign_checkout()).unwrap();

        assert!(path.exists());
        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            before,
            "the probe must not touch an object's mtime"
        );
        // and no scratch file is left behind in the store
        for fanout in fs::read_dir(dir.path().join(".git").join("objects"))
            .unwrap()
            .flatten()
        {
            for entry in fs::read_dir(fanout.path()).into_iter().flatten().flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                assert!(!name.starts_with("tmp_joy_"), "left {name} behind");
            }
        }
    }
}
