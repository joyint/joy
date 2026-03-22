// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Embedded file sync: hash-based diff and install for files shipped inside the Joy binary.

use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::JoyError;
use crate::store;

/// An embedded file that ships with the Joy binary.
pub struct EmbeddedFile {
    /// Content (from `include_str!`).
    pub content: &'static str,
    /// Target path relative to `.joy/` (e.g. `hooks/commit-msg`).
    pub target: &'static str,
    /// Whether the file should be executable (Unix only).
    pub executable: bool,
}

/// Status of an installed file compared to the embedded version.
#[derive(Debug, PartialEq, Eq)]
pub enum FileStatus {
    UpToDate,
    Outdated,
    Missing,
}

/// Result of a sync operation for one file.
#[derive(Debug)]
pub struct SyncAction {
    pub target: &'static str,
    pub action: &'static str, // "updated", "created", "up to date"
}

fn sha256_hex(data: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Compare installed files against embedded versions.
/// Returns a list of (target_path, status) for files that are not up to date.
pub fn diff_files(
    root: &Path,
    files: &[EmbeddedFile],
) -> Result<Vec<(&'static str, FileStatus)>, JoyError> {
    let joy_dir = store::joy_dir(root);
    let mut results = Vec::new();

    for file in files {
        let installed_path = joy_dir.join(file.target);
        let expected_hash = sha256_hex(file.content);

        let status = if installed_path.is_file() {
            let installed =
                fs::read_to_string(&installed_path).map_err(|e| JoyError::ReadFile {
                    path: installed_path.clone(),
                    source: e,
                })?;
            if sha256_hex(&installed) == expected_hash {
                FileStatus::UpToDate
            } else {
                FileStatus::Outdated
            }
        } else {
            FileStatus::Missing
        };

        results.push((file.target, status));
    }

    Ok(results)
}

/// Sync embedded files to disk. Only writes files that are outdated or missing.
/// Returns a list of actions taken.
pub fn sync_files(root: &Path, files: &[EmbeddedFile]) -> Result<Vec<SyncAction>, JoyError> {
    let joy_dir = store::joy_dir(root);
    let diffs = diff_files(root, files)?;
    let mut actions = Vec::new();

    for (file, (_target, status)) in files.iter().zip(diffs.iter()) {
        let action = match status {
            FileStatus::UpToDate => {
                actions.push(SyncAction {
                    target: file.target,
                    action: "up to date",
                });
                continue;
            }
            FileStatus::Outdated => "updated",
            FileStatus::Missing => "created",
        };

        let installed_path = joy_dir.join(file.target);
        if let Some(parent) = installed_path.parent() {
            fs::create_dir_all(parent).map_err(|e| JoyError::CreateDir {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }

        fs::write(&installed_path, file.content).map_err(|e| JoyError::WriteFile {
            path: installed_path.clone(),
            source: e,
        })?;

        #[cfg(unix)]
        if file.executable {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o755);
            fs::set_permissions(&installed_path, perms).map_err(|e| JoyError::WriteFile {
                path: installed_path,
                source: e,
            })?;
        }

        actions.push(SyncAction {
            target: file.target,
            action,
        });
    }

    Ok(actions)
}

/// Check if all files are up to date (no outdated or missing files).
pub fn all_up_to_date(root: &Path, files: &[EmbeddedFile]) -> Result<bool, JoyError> {
    let diffs = diff_files(root, files)?;
    Ok(diffs.iter().all(|(_, s)| *s == FileStatus::UpToDate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup_project(dir: &Path) {
        let joy_dir = dir.join(".joy");
        fs::create_dir_all(&joy_dir).unwrap();
        fs::write(joy_dir.join("project.yaml"), "name: test\nacronym: TP\n").unwrap();
        fs::write(joy_dir.join("config.defaults.yaml"), "version: 1\n").unwrap();
    }

    #[test]
    fn diff_missing_file() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let files = [EmbeddedFile {
            content: "hello",
            target: "test/file.txt",
            executable: false,
        }];

        let diffs = diff_files(dir.path(), &files).unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].1, FileStatus::Missing);
    }

    #[test]
    fn sync_creates_and_reports() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let files = [EmbeddedFile {
            content: "hello",
            target: "test/file.txt",
            executable: false,
        }];

        let actions = sync_files(dir.path(), &files).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action, "created");

        // Second sync should be up to date
        let actions = sync_files(dir.path(), &files).unwrap();
        assert_eq!(actions[0].action, "up to date");
    }

    #[test]
    fn sync_detects_outdated() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let files = [EmbeddedFile {
            content: "new content",
            target: "test/file.txt",
            executable: false,
        }];

        // Write old content
        let path = dir.path().join(".joy/test");
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("file.txt"), "old content").unwrap();

        let diffs = diff_files(dir.path(), &files).unwrap();
        assert_eq!(diffs[0].1, FileStatus::Outdated);

        let actions = sync_files(dir.path(), &files).unwrap();
        assert_eq!(actions[0].action, "updated");

        // Verify content was updated
        let content = fs::read_to_string(path.join("file.txt")).unwrap();
        assert_eq!(content, "new content");
    }

    #[test]
    fn all_up_to_date_check() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let files = [EmbeddedFile {
            content: "hello",
            target: "test/file.txt",
            executable: false,
        }];

        assert!(!all_up_to_date(dir.path(), &files).unwrap());
        sync_files(dir.path(), &files).unwrap();
        assert!(all_up_to_date(dir.path(), &files).unwrap());
    }
}
