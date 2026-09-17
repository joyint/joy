// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! THE cross-process advisory whole-file lock (design D2.6a).
//!
//! joy had none. [`crate::vcs::forge::checkout_gate`] is a per-process
//! map of mutexes and says so itself, and a second joy process on the
//! same machine simply did not see it. Two things now need more than
//! that: appending a line to `known_hosts` (a read-modify-write on a
//! file other processes read) and refreshing a forge token (where two
//! refreshes at once killed Codeberg tokens for an hour on 2026-08-29).
//! Both take this lock, so exactly one crate-level dependency carries
//! file locking for all of joy.
//!
//! What the operating systems promise, because the callers depend on it:
//!
//! - unix: `flock(2)`, which is ADVISORY. A process that does not ask
//!   for the lock still reads and writes the file. The lock belongs to
//!   the open file description, so two handles in one process contend
//!   with each other, and a forked child that unlocks takes the
//!   parent's lock away with it. Hold it for a read-modify-write and
//!   for nothing else: never spawn a child while holding it.
//! - Windows: `LockFileEx` with `LOCKFILE_EXCLUSIVE_LOCK` over the
//!   whole range, which is MANDATORY for that range ("denies all other
//!   processes both read and write access to the specified region").
//!   It is released when the handle closes or the process dies, but
//!   "the time it takes for the operating system to unlock these locks
//!   depends upon available system resources", which is why every
//!   caller waits with a bound and re-reads afterwards instead of
//!   trusting the lock alone. `LockFileEx` works over SMB 3.0, so a
//!   redirected `%LOCALAPPDATA%` still locks.
//!
//! The lock file itself is never unlinked: unlinking it while another
//! process holds it open hands the next caller a different file under
//! the same name, and the two would not see each other at all.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fs4::FileExt;

/// The pause between two non-blocking attempts. Short enough that a
/// refresh behind a lock is not felt, long enough that a ten-second
/// wait is two hundred syscalls and not a spin.
const BACKOFF: Duration = Duration::from_millis(50);

/// The wait every joy caller uses unless it has a reason of its own
/// (D2.6a: "a non blocking attempt in a 50 ms backoff loop, 10 s
/// total").
pub const DEFAULT_WAIT: Duration = Duration::from_secs(10);

/// Why the lock did not come.
#[derive(Debug)]
pub enum LockError {
    /// Somebody else holds it and the wait ran out. The caller does
    /// NOT do the work anyway: it re-reads and reports "busy".
    Busy { path: PathBuf, waited: Duration },
    /// The lock file could not be opened, or the operating system
    /// refused the lock itself (a filesystem without locking, say).
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Busy { path, waited } => write!(
                f,
                "another process is holding {} (waited {} ms)",
                path.display(),
                waited.as_millis()
            ),
            LockError::Io { path, source } => {
                write!(f, "cannot lock {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for LockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LockError::Busy { .. } => None,
            LockError::Io { source, .. } => Some(source),
        }
    }
}

/// A held exclusive lock. Dropping it releases the lock; the file
/// stays.
#[derive(Debug)]
pub struct FileLock {
    file: File,
    path: PathBuf,
}

impl FileLock {
    /// The lock file this lock is held on.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        // The handle closing would release it anyway; asking first
        // makes the release a syscall with an error we can see rather
        // than a side effect of the close.
        if let Err(e) = FileExt::unlock(&self.file) {
            tracing::debug!(path = %self.path.display(), error = %e, "file lock not released cleanly");
        }
    }
}

/// Take the exclusive lock on `path`, waiting at most `wait`.
///
/// The file is created when it is missing (and its parent directory
/// with it), 0600 on unix: it is joy's own state and holds nothing but
/// the lock. An existing file is never truncated, so a caller may keep
/// content in it.
pub fn exclusive(path: &Path, wait: Duration) -> Result<FileLock, LockError> {
    let io = |source: std::io::Error| LockError::Io {
        path: path.to_path_buf(),
        source,
    };
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
    }
    let file = open_lock_file(path).map_err(io)?;
    let started = Instant::now();
    loop {
        // Named through the trait on purpose: `std::fs::File` grew its
        // own inherent `try_lock` in Rust 1.89, and an inherent method
        // wins over a trait method, so `file.try_lock()` would quietly
        // stop being the fs4 call this module documents.
        match FileExt::try_lock(&file) {
            Ok(()) => {
                return Ok(FileLock {
                    file,
                    path: path.to_path_buf(),
                })
            }
            Err(fs4::TryLockError::WouldBlock) => {}
            Err(fs4::TryLockError::Error(e)) => return Err(io(e)),
        }
        if started.elapsed() >= wait {
            return Err(LockError::Busy {
                path: path.to_path_buf(),
                waited: started.elapsed(),
            });
        }
        std::thread::sleep(BACKOFF.min(wait.saturating_sub(started.elapsed())));
    }
}

#[cfg(unix)]
fn open_lock_file(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_lock_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_taker_waits_and_then_says_busy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locks").join("one.lock");
        let held = exclusive(&path, DEFAULT_WAIT).expect("first lock");
        // A second handle in the same process contends exactly like a
        // second process does: flock keys on the open file description,
        // LockFileEx on the handle.
        let started = Instant::now();
        let refused = exclusive(&path, Duration::from_millis(200));
        match refused {
            Err(LockError::Busy { .. }) => {}
            other => panic!("expected Busy, got {other:?}"),
        }
        assert!(
            started.elapsed() >= Duration::from_millis(180),
            "the wait was not honoured: {:?}",
            started.elapsed()
        );
        drop(held);
        // Released: the next taker gets it at once.
        let again = exclusive(&path, Duration::from_millis(200)).expect("lock after release");
        assert_eq!(again.path(), path);
    }

    #[test]
    fn the_lock_file_survives_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("two.lock");
        {
            let _lock = exclusive(&path, DEFAULT_WAIT).unwrap();
        }
        assert!(path.exists(), "the lock file must never be unlinked");
    }

    #[test]
    fn content_in_the_lock_file_is_not_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("three.lock");
        std::fs::write(&path, b"held by nobody").unwrap();
        let lock = exclusive(&path, DEFAULT_WAIT).unwrap();
        drop(lock);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "held by nobody");
    }
}
