// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The refresh lock (D2.6a).
//!
//! Two refreshes of one rotating refresh token at once killed Codeberg
//! tokens for an hour on 2026-08-29, and ten thousand attempts against
//! one dead GitHub refresh token got a whole OAuth app throttled on
//! 2026-09-07. Both are the same shape: several processes, one entry,
//! no lock. Because D2.6 makes the connector the only process that
//! touches joy's own entry, the lock lives here and is taken by every
//! `token`, `login`, `token-store` and `logout` call that may write.
//!
//! The primitive is joy's one cross process advisory whole file lock,
//! `joy_core::util::file_lock`, which landed with J4a in wave 0. No
//! package after it carries a lock dependency of its own.
//!
//! The protocol, in the order D2.6a writes it:
//!
//! 1. open or create the lock file;
//! 2. take the exclusive lock with a bounded wait (50 ms backoff, 10 s);
//! 3. re read the entry;
//! 4. refresh only if it still carries the same access token
//!    fingerprint and is still expired against a 60 s skew;
//! 5. write, release by dropping the handle, never unlink the file.
//!
//! On a timeout the caller does NOT refresh: it re reads once, uses the
//! entry if it is now valid, and otherwise answers `busy`. "Refresh
//! anyway" is the failure this module exists to prevent.
//!
//! One more rule, from flock's own manual: "If a process holding a lock
//! on a file forks and the child explicitly unlocks the file, the
//! parent will lose its lock". The connector therefore reads gh, glab
//! and tea answers BEFORE it takes the lock and never spawns them while
//! holding it.

use std::path::PathBuf;

use joy_core::util::file_lock::{self, FileLock, LockError};

/// Why a lock attempt gave nothing.
#[derive(Debug)]
pub enum Busy {
    /// Somebody else held it for the whole wait.
    Held,
    /// The lock could not be taken at all (a filesystem without
    /// locking, an unwritable state directory). D2.6a: degrade to "do
    /// not refresh, use the entry as it stands, report busy", never to
    /// "refresh anyway".
    Unavailable(String),
}

impl std::fmt::Display for Busy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Busy::Held => write!(f, "another joy process is writing this credential"),
            Busy::Unavailable(reason) => write!(f, "this credential could not be locked: {reason}"),
        }
    }
}

/// The lock file of one host and login: `forge-<first 16 hex of
/// SHA256(host|login)>.lock` under the person's app state directory.
///
/// Not under the configuration directory, because joy's config base is
/// `%APPDATA%` on Windows, which roams: a lock that travels to another
/// machine locks nothing there and is a lie here.
pub fn lock_path(
    state_dir: Option<&std::path::Path>,
    host: &str,
    login: Option<&str>,
) -> Option<PathBuf> {
    use sha2::{Digest, Sha256};
    let base = match state_dir {
        Some(dir) => dir.to_path_buf(),
        None => joy_core::auth::session::app_state_dir().ok()?,
    };
    let key = format!("{host}|{}", login.unwrap_or_default());
    let digest = hex::encode(Sha256::digest(key.as_bytes()));
    Some(
        base.join("locks")
            .join(format!("forge-{}.lock", &digest[..16])),
    )
}

/// Take the refresh lock for one host and login, waiting at most the
/// 10 s of D2.6a. `state_dir` is the person's app state directory, or
/// the one a test named.
pub fn take(
    state_dir: Option<&std::path::Path>,
    host: &str,
    login: Option<&str>,
) -> Result<FileLock, Busy> {
    let Some(path) = lock_path(state_dir, host, login) else {
        return Err(Busy::Unavailable(
            "joy cannot find this person's app state directory".to_string(),
        ));
    };
    take_at(&path)
}

/// [`take`] on a named file, for the tests and for a caller that
/// already computed the path.
pub fn take_at(path: &std::path::Path) -> Result<FileLock, Busy> {
    match file_lock::exclusive(path, file_lock::DEFAULT_WAIT) {
        Ok(lock) => Ok(lock),
        Err(LockError::Busy { .. }) => Err(Busy::Held),
        Err(error @ LockError::Io { .. }) => Err(Busy::Unavailable(error.to_string())),
    }
}

/// The answer of D2.4 for a call that could not take the lock and found
/// nothing usable when it looked again.
pub fn busy_answer() -> serde_json::Value {
    serde_json::json!({ "known": false, "reason": "busy" })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The name is a fingerprint of host and login, so two logins on
    /// one host do not serialise against each other and no host name
    /// ever reaches a file system as itself.
    #[test]
    fn the_lock_file_is_named_by_the_digest_of_host_and_login() {
        let Some(a) = lock_path(None, "codeberg.org", Some("scotty")) else {
            return; // no app state directory on this machine
        };
        let b = lock_path(None, "codeberg.org", Some("work")).unwrap();
        let c = lock_path(None, "codeberg.org", Some("scotty")).unwrap();
        assert_ne!(a, b, "two logins take two locks");
        assert_eq!(a, c, "the same pair takes the same lock");
        let name = a.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("forge-"), "{name}");
        assert!(name.ends_with(".lock"), "{name}");
        assert_eq!(name.len(), "forge-".len() + 16 + ".lock".len());
        assert!(!name.contains("codeberg"), "no host name in the file name");
        assert!(a.parent().unwrap().ends_with("locks"));
    }

    /// The whole point: the second taker does not refresh. It waits,
    /// gives up and says busy.
    #[test]
    fn a_second_taker_is_told_busy_and_never_told_to_refresh_anyway() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locks").join("one.lock");
        let held = take_at(&path).expect("the first lock");
        let path_two = path.clone();
        // A short wait, because the point is the ANSWER and not the ten
        // seconds: `file_lock::exclusive` proves the bound itself.
        let refused =
            joy_core::util::file_lock::exclusive(&path_two, std::time::Duration::from_millis(120));
        assert!(matches!(refused, Err(LockError::Busy { .. })));
        drop(held);
        let again = take_at(&path).expect("the lock after the release");
        drop(again);
        assert!(path.exists(), "the lock file is never unlinked");
    }

    #[test]
    fn the_busy_answer_has_the_shape_d2_4_names() {
        let answer = busy_answer();
        assert_eq!(answer["known"], false);
        assert_eq!(answer["reason"], "busy");
    }
}
