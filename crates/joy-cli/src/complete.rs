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

/// Filter member ids down to AI members whose id starts with `prefix`,
/// returned sorted for deterministic completion output.
fn filter_ai_members<'a, I: Iterator<Item = &'a String>>(members: I, prefix: &str) -> Vec<String> {
    let mut out: Vec<String> = members
        .filter(|id| joy_core::model::project::is_ai_member(id))
        .filter(|id| id.starts_with(prefix))
        .cloned()
        .collect();
    out.sort();
    out
}

/// Complete AI member ids from the project's member list.
///
/// Scans `project.yaml` for entries whose id starts with `ai:` and filters
/// by the current prefix. Errors (missing project, unreadable yaml) yield
/// no candidates rather than panicking - completion must always be
/// non-fatal for the interactive shell.
pub fn complete_ai_member(current: &OsStr) -> Vec<CompletionCandidate> {
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
    let project = match store::load_project(&root) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    filter_ai_members(project.members.keys(), prefix)
        .into_iter()
        .map(CompletionCandidate::new)
        .collect()
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

/// Known config keys from the Config struct.
const STATIC_CONFIG_KEYS: &[&str] = &[
    "version",
    "output.color",
    "output.emoji",
    "output.short",
    "output.fortune",
    "output.fortune-category",
    "sync.remote",
    "sync.auto",
    "ai.tool",
    "ai.command",
    "ai.model",
    "ai.max_cost_per_job",
    "ai.currency",
    "modes.default",
];

/// Complete config keys for `joy config get/set`.
pub fn complete_config_key(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(prefix) = current.to_str() else {
        return Vec::new();
    };

    let mut candidates: Vec<CompletionCandidate> = STATIC_CONFIG_KEYS
        .iter()
        .filter(|k| k.starts_with(prefix))
        .map(|k| CompletionCandidate::new(*k))
        .collect();

    // Add dynamic agent role keys from current config
    let config_value = store::load_config_value();
    if let Some(agents) = config_value.get("agents").and_then(|a| a.as_object()) {
        for role in agents.keys() {
            if role != "default" {
                let key = format!("agents.{role}.mode");
                if key.starts_with(prefix) {
                    candidates.push(CompletionCandidate::new(key));
                }
            }
        }
    }

    candidates
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

    #[test]
    fn filter_ai_members_only_returns_ai_prefix() {
        let members: Vec<String> = vec![
            "horst@joydev.com".into(),
            "ai:claude@joy".into(),
            "ai:qwen@joy".into(),
            "alice@team.com".into(),
        ];
        let out = filter_ai_members(members.iter(), "");
        assert_eq!(out, vec!["ai:claude@joy", "ai:qwen@joy"]);
    }

    #[test]
    fn filter_ai_members_respects_prefix() {
        let members: Vec<String> = vec![
            "ai:claude@joy".into(),
            "ai:qwen@joy".into(),
            "ai:copilot@joy".into(),
        ];
        let out = filter_ai_members(members.iter(), "ai:c");
        assert_eq!(out, vec!["ai:claude@joy", "ai:copilot@joy"]);
    }

    #[test]
    fn filter_ai_members_sorted_output() {
        let members: Vec<String> = vec![
            "ai:zzz@joy".into(),
            "ai:aaa@joy".into(),
            "ai:mmm@joy".into(),
        ];
        let out = filter_ai_members(members.iter(), "");
        assert_eq!(out, vec!["ai:aaa@joy", "ai:mmm@joy", "ai:zzz@joy"]);
    }

    #[test]
    fn filter_ai_members_empty_on_no_match() {
        let members: Vec<String> = vec!["ai:claude@joy".into()];
        let out = filter_ai_members(members.iter(), "ai:x");
        assert!(out.is_empty());
    }
}
