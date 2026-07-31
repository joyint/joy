// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Strip the stored `history` list from item files.
//!
//! The audit trail lives in the event log alone — (timestamp, id,
//! action, actor), append-only, value-free (decision JOY-0175-9B); every
//! display derives the "Updated" trail from it at lookup time. The
//! `history:` lists an interim build stored on items duplicated exactly
//! that record, so this reconcile removes the key wherever it was
//! written. Encrypted items cannot be rewritten without their zone key;
//! their stray key is ignored by the reader and drops on their next
//! authored save.

use std::path::Path;

use super::Reconciled;
use crate::error::JoyError;

const KEY: &str = ".joy/items history";
const TO: &str = "stored history stripped (the event log is the one audit record, JOY-0175-9B)";

fn files_with_history(root: &Path) -> Vec<std::path::PathBuf> {
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
                continue; // sealed: dropped on the next authored save
            }
            let Ok(value) = serde_yaml_ng::from_slice::<serde_yaml_ng::Value>(&bytes) else {
                continue;
            };
            if value.get("history").is_some() {
                out.push(path);
            }
        }
    }
    out
}

/// Read-only: whether any plaintext item file still stores `history`.
pub fn pending(root: &Path) -> Result<Vec<Reconciled>, JoyError> {
    if files_with_history(root).is_empty() {
        Ok(Vec::new())
    } else {
        Ok(vec![Reconciled { key: KEY, to: TO }])
    }
}

/// Remove the `history` key from every plaintext item file carrying it.
pub fn migrate(root: &Path) -> Result<Vec<Reconciled>, JoyError> {
    let files = files_with_history(root);
    if files.is_empty() {
        return Ok(Vec::new());
    }
    for path in &files {
        // Textual block removal, not a parse-and-reserialize round trip:
        // everything the migration does NOT touch stays byte-identical,
        // so the sync commit shows exactly the removed lines per item.
        let text = std::fs::read_to_string(path).map_err(|e| JoyError::ReadFile {
            path: path.clone(),
            source: e,
        })?;
        let mut out = String::with_capacity(text.len());
        let mut in_history = false;
        for line in text.lines() {
            if line.starts_with("history:") {
                in_history = true;
                continue;
            }
            if in_history {
                // the block's entries are indented or list items; the
                // next top-level key ends it
                let continues = line.starts_with(' ') || line.starts_with('-') || line.is_empty();
                if continues {
                    continue;
                }
                in_history = false;
            }
            out.push_str(line);
            out.push('\n');
        }
        std::fs::write(path, out).map_err(|e| JoyError::WriteFile {
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
    fn strips_only_files_with_history_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let items = dir.path().join(".joy").join("items");
        std::fs::create_dir_all(&items).unwrap();
        std::fs::write(
            items.join("a.yaml"),
            "id: T-1\ntitle: audited\nhistory:\n- date: 2026-01-01T00:00:00Z\n  by: a@x\n",
        )
        .unwrap();
        std::fs::write(items.join("b.yaml"), "id: T-2\ntitle: clean\n").unwrap();

        assert_eq!(pending(dir.path()).unwrap().len(), 1);
        assert_eq!(migrate(dir.path()).unwrap().len(), 1);
        let a = std::fs::read_to_string(items.join("a.yaml")).unwrap();
        assert!(!a.contains("history"), "{a}");
        assert!(a.contains("title: audited"), "{a}");

        assert!(pending(dir.path()).unwrap().is_empty());
        assert!(migrate(dir.path()).unwrap().is_empty());
    }
}
