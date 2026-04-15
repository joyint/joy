// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Single-use delegation token tracking (ADR-033).
//!
//! `joy auth --token` records each successfully redeemed token's id in
//! `~/.local/state/joy/consumed-tokens.json` so that a second redemption
//! attempt for the same token id is rejected as replay.
//!
//! Entries are garbage-collected lazily on every write: once the stored
//! token expiry is in the past, the entry can be dropped because any
//! replay attempt would be rejected by the expiry check anyway.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::session::dirs_state_dir;
use crate::error::JoyError;

/// A single consumed token entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsumedEntry {
    pub token_id: String,
    pub redeemed_at: DateTime<Utc>,
    /// Token expiry captured from the DelegationClaims. Entries with
    /// `expires_at` in the past can be GC'd because the expiry check
    /// would reject the replay before the consumed check is reached.
    /// For tokens issued without expiry, this is None and the entry is
    /// kept indefinitely.
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ConsumedFile {
    #[serde(default)]
    entries: Vec<ConsumedEntry>,
}

fn consumed_path() -> Result<PathBuf, JoyError> {
    Ok(dirs_state_dir()?.join("joy").join("consumed-tokens.json"))
}

fn load() -> Result<ConsumedFile, JoyError> {
    let path = consumed_path()?;
    if !path.exists() {
        return Ok(ConsumedFile::default());
    }
    let data = std::fs::read_to_string(&path).map_err(|e| JoyError::ReadFile {
        path: path.clone(),
        source: e,
    })?;
    // Tolerate a corrupt or empty file: start over rather than locking the
    // user out of fresh auth because of an unrelated on-disk mishap.
    Ok(serde_json::from_str(&data).unwrap_or_default())
}

fn save(file: &ConsumedFile) -> Result<(), JoyError> {
    let path = consumed_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| JoyError::CreateDir {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    let data = serde_json::to_string_pretty(file).expect("consumed tokens serialize");
    std::fs::write(&path, data).map_err(|e| JoyError::WriteFile {
        path: path.clone(),
        source: e,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(&path, perms);
    }
    Ok(())
}

fn gc(file: &mut ConsumedFile) {
    let now = Utc::now();
    file.entries
        .retain(|e| e.expires_at.map(|exp| exp > now).unwrap_or(true));
}

/// Return the redemption timestamp if this token_id has already been consumed.
pub fn is_consumed(token_id: &str) -> Result<Option<DateTime<Utc>>, JoyError> {
    let file = load()?;
    Ok(file
        .entries
        .iter()
        .find(|e| e.token_id == token_id)
        .map(|e| e.redeemed_at))
}

/// Record a successful redemption. Lazily garbage-collects entries whose
/// token expiry has passed.
pub fn mark_consumed(token_id: &str, expires_at: Option<DateTime<Utc>>) -> Result<(), JoyError> {
    let mut file = load()?;
    gc(&mut file);
    if !file.entries.iter().any(|e| e.token_id == token_id) {
        file.entries.push(ConsumedEntry {
            token_id: token_id.to_string(),
            redeemed_at: Utc::now(),
            expires_at,
        });
    }
    save(&file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use tempfile::tempdir;

    fn with_state_dir<F: FnOnce()>(f: F) {
        let _guard = super::super::STATE_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        // SAFETY: serialized via ENV_LOCK
        unsafe { std::env::set_var("XDG_STATE_HOME", dir.path()) };
        f();
        // SAFETY: serialized via ENV_LOCK
        unsafe { std::env::remove_var("XDG_STATE_HOME") };
    }

    #[test]
    fn fresh_token_is_not_consumed() {
        with_state_dir(|| {
            assert!(is_consumed("unseen").unwrap().is_none());
        });
    }

    #[test]
    fn marking_consumed_is_observable() {
        with_state_dir(|| {
            mark_consumed("abc", Some(Utc::now() + Duration::hours(2))).unwrap();
            assert!(is_consumed("abc").unwrap().is_some());
        });
    }

    #[test]
    fn remarking_same_id_is_idempotent() {
        with_state_dir(|| {
            let exp = Utc::now() + Duration::hours(2);
            mark_consumed("abc", Some(exp)).unwrap();
            mark_consumed("abc", Some(exp)).unwrap();
            let file = load().unwrap();
            let matches: Vec<&ConsumedEntry> = file
                .entries
                .iter()
                .filter(|e| e.token_id == "abc")
                .collect();
            assert_eq!(matches.len(), 1);
        });
    }

    #[test]
    fn gc_drops_expired_entries_on_write() {
        with_state_dir(|| {
            let past = Utc::now() - Duration::hours(3);
            let future = Utc::now() + Duration::hours(1);
            // Seed the log manually so the "expired" entry has a timestamp
            // that is unambiguously in the past when we next write.
            let mut file = ConsumedFile::default();
            file.entries.push(ConsumedEntry {
                token_id: "old".into(),
                redeemed_at: past - Duration::hours(1),
                expires_at: Some(past),
            });
            file.entries.push(ConsumedEntry {
                token_id: "new".into(),
                redeemed_at: Utc::now(),
                expires_at: Some(future),
            });
            save(&file).unwrap();

            // Any new mark_consumed triggers GC of the expired entry.
            mark_consumed("fresh", Some(future)).unwrap();
            let file = load().unwrap();
            let ids: Vec<&str> = file.entries.iter().map(|e| e.token_id.as_str()).collect();
            assert!(!ids.contains(&"old"), "expired entry should be GC'd");
            assert!(ids.contains(&"new"));
            assert!(ids.contains(&"fresh"));
        });
    }

    #[test]
    fn entry_without_expiry_is_kept() {
        with_state_dir(|| {
            mark_consumed("no-expiry", None).unwrap();
            mark_consumed("other", Some(Utc::now() + Duration::hours(1))).unwrap();
            assert!(is_consumed("no-expiry").unwrap().is_some());
        });
    }
}
