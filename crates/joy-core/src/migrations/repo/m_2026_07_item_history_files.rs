// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Persist the item `history` backfill into the files.
//!
//! The on-read schema migration (`migrations::item_yaml`) supplies the
//! empty audit list; this reconcile writes it into every plaintext item
//! and job file that still lacks the key, so the stored shape matches
//! what the strict model requires. Encrypted items stay untouched — no
//! filesystem pass can rewrite them without their zone key — and keep
//! being covered by the on-read layer.

use std::path::Path;

use super::Reconciled;
use crate::error::JoyError;

const KEY: &str = ".joy/items history";
const TO: &str = "history: [] backfilled (strict item schema)";

fn files_missing_history(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for dir in ["items", "jobs"] {
        let dir = crate::store::joy_dir(root).join(dir);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            if joy_crypt::zone::looks_like_blob(&bytes) {
                continue; // sealed: the on-read layer covers it
            }
            let Ok(value) = serde_yaml_ng::from_slice::<serde_yaml_ng::Value>(&bytes) else {
                continue;
            };
            if value.get("history").is_none() {
                out.push(path);
            }
        }
    }
    out
}

/// Read-only: whether any plaintext item file still lacks `history`.
pub fn pending(root: &Path) -> Result<Vec<Reconciled>, JoyError> {
    if files_missing_history(root).is_empty() {
        Ok(Vec::new())
    } else {
        Ok(vec![Reconciled { key: KEY, to: TO }])
    }
}

/// Append `history: []` to every plaintext item file missing it.
pub fn migrate(root: &Path) -> Result<Vec<Reconciled>, JoyError> {
    let files = files_missing_history(root);
    if files.is_empty() {
        return Ok(Vec::new());
    }
    for path in &files {
        // Append the key textually instead of a parse-and-reserialize
        // round trip: the recorded formatting of every other field stays
        // byte-identical, so the sync commit shows one added line per
        // item and nothing else.
        let mut text = std::fs::read_to_string(path).map_err(|e| JoyError::ReadFile {
            path: path.clone(),
            source: e,
        })?;
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str("history: []\n");
        std::fs::write(path, text).map_err(|e| JoyError::WriteFile {
            path: path.clone(),
            source: e,
        })?;
    }
    let rel_items = format!("{}/items", crate::store::JOY_DIR);
    let rel_jobs = format!("{}/jobs", crate::store::JOY_DIR);
    crate::git_ops::auto_git_add(root, &[&rel_items, &rel_jobs]);
    Ok(vec![Reconciled { key: KEY, to: TO }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backfills_only_files_without_history_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let items = dir.path().join(".joy").join("items");
        std::fs::create_dir_all(&items).unwrap();
        std::fs::write(items.join("a.yaml"), "id: T-1\ntitle: old\n").unwrap();
        std::fs::write(items.join("b.yaml"), "id: T-2\ntitle: new\nhistory: []\n").unwrap();

        assert_eq!(pending(dir.path()).unwrap().len(), 1);
        assert_eq!(migrate(dir.path()).unwrap().len(), 1);
        let a = std::fs::read_to_string(items.join("a.yaml")).unwrap();
        assert!(a.ends_with("history: []\n"), "{a}");
        let b = std::fs::read_to_string(items.join("b.yaml")).unwrap();
        assert_eq!(b.matches("history").count(), 1);

        assert!(pending(dir.path()).unwrap().is_empty());
        assert!(migrate(dir.path()).unwrap().is_empty());
    }
}
