// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Dynamic shell completion for item and milestone IDs.

use std::ffi::OsStr;

use clap_complete::engine::CompletionCandidate;

use joy_core::store;

/// Complete item IDs by scanning .joy/items/ filenames.
pub fn complete_item_id(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(prefix) = current.to_str() else {
        return Vec::new();
    };

    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let Some(root) = store::find_project_root(&cwd) else {
        return Vec::new();
    };

    let items_dir = store::joy_dir(&root).join(store::ITEMS_DIR);
    let ms_dir = store::joy_dir(&root).join(store::MILESTONES_DIR);

    let mut candidates = Vec::new();

    // Scan items
    if let Ok(entries) = std::fs::read_dir(&items_dir) {
        for entry in entries.flatten() {
            if let Some(id) = extract_id(&entry.file_name()) {
                if id.starts_with(prefix) {
                    candidates.push(CompletionCandidate::new(id));
                }
            }
        }
    }

    // Scan milestones
    if let Ok(entries) = std::fs::read_dir(&ms_dir) {
        for entry in entries.flatten() {
            if let Some(id) = extract_id(&entry.file_name()) {
                if id.starts_with(prefix) {
                    candidates.push(CompletionCandidate::new(id));
                }
            }
        }
    }

    candidates.sort_by(|a, b| a.get_value().cmp(b.get_value()));
    candidates
}

/// Extract the ID prefix from a filename like "JOY-0001-some-title.yaml".
fn extract_id(filename: &OsStr) -> Option<String> {
    let name = filename.to_str()?;
    let stem = name.strip_suffix(".yaml")?;
    // ID is the part before the first dash after the acronym-number pattern
    // e.g. "JOY-0001-some-title" -> "JOY-0001"
    // e.g. "JOY-MS-01-some-title" -> "JOY-MS-01"
    let parts: Vec<&str> = stem.splitn(4, '-').collect();
    match parts.len() {
        // ACR-XXXX-... -> ACR-XXXX
        2.. => {
            // Check if second part is "MS" (milestone)
            if parts.len() >= 3 && parts[1] == "MS" {
                Some(format!("{}-MS-{}", parts[0], parts[2]))
            } else {
                Some(format!("{}-{}", parts[0], parts[1]))
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn extract_item_id() {
        let f = OsString::from("JOY-0001-login-page.yaml");
        assert_eq!(extract_id(&f), Some("JOY-0001".to_string()));
    }

    #[test]
    fn extract_milestone_id() {
        let f = OsString::from("JOY-MS-01-mvp-release.yaml");
        assert_eq!(extract_id(&f), Some("JOY-MS-01".to_string()));
    }

    #[test]
    fn extract_no_yaml() {
        let f = OsString::from("README.md");
        assert_eq!(extract_id(&f), None);
    }
}
