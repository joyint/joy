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

    let mut all_ids: Vec<String> = Vec::new();
    for dir in [&items_dir, &ms_dir] {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if let Some(id) = extract_id(&entry.file_name()) {
                    all_ids.push(id);
                }
            }
        }
    }

    let mut candidates: Vec<CompletionCandidate> = all_ids
        .iter()
        .filter_map(|id| rewrite_candidate(id, prefix))
        .map(CompletionCandidate::new)
        .collect();

    candidates.sort_by(|a, b| a.get_value().cmp(b.get_value()));
    candidates
}

/// Match an item ID against a completion prefix using three rules in
/// priority order, and return the canonical ID with the matched substring
/// overwritten by the user's exact (potentially differently-cased) prefix.
///
/// Rules (case-insensitive):
/// 1. Direct prefix on the full ID. `J` matches `JOY-...` and `JI-...`;
///    `JOY-00` matches `JOY-00xx-...`.
/// 2. Direct prefix on the part after the first dash. `00` matches
///    `JOY-0001`, `JI-0042-AB`. `MS` matches `JOY-MS-01`.
/// 3. Substring inside the part after the first dash. `AA`/`aa` match
///    `JOY-00AA-1B` via the hex chunk.
///
/// The returned candidate contains the user's prefix verbatim where the
/// match landed, so case-sensitive shell completers (e.g. zsh `_describe`)
/// still accept it. Joy resolves IDs case-insensitively at the command
/// layer, so a candidate like `JOY-013f-33` resolves to `JOY-013F-33`.
fn rewrite_candidate(id: &str, prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        return Some(id.to_string());
    }
    let id_lc = id.to_ascii_lowercase();
    let prefix_lc = prefix.to_ascii_lowercase();
    if id_lc.starts_with(&prefix_lc) {
        return Some(splice(id, 0, prefix.len(), prefix));
    }
    if let Some((acronym, after)) = id.split_once('-') {
        let after_lc = after.to_ascii_lowercase();
        let after_offset = acronym.len() + 1;
        if after_lc.starts_with(&prefix_lc) {
            return Some(splice(id, after_offset, prefix.len(), prefix));
        }
        if let Some(pos) = after_lc.find(&prefix_lc) {
            return Some(splice(id, after_offset + pos, prefix.len(), prefix));
        }
    }
    None
}

/// Replace `len` bytes starting at byte index `start` in `s` with `replacement`.
/// All inputs are ASCII in the completer's call sites, so byte indexing is safe.
fn splice(s: &str, start: usize, len: usize, replacement: &str) -> String {
    let mut out = String::with_capacity(s.len() - len + replacement.len());
    out.push_str(&s[..start]);
    out.push_str(replacement);
    out.push_str(&s[start + len..]);
    out
}

#[cfg(test)]
fn matches_item_id(id: &str, prefix: &str) -> bool {
    rewrite_candidate(id, prefix).is_some()
}

/// Match a member ID against a completion prefix, handling colon as a
/// possible word boundary that the shell may have stripped (bash's
/// COMP_WORDBREAKS includes `:`). Matching is case-insensitive on both
/// branches so `p` finds `Peter.Schmidt@joydev.com` without forcing the
/// user to type uppercase.
///
/// Cases (id = `ai:claude@joy`):
/// * prefix `ai`         -> `ai:claude@joy` (full)
/// * prefix `ai:cl`      -> `ai:claude@joy` (full, prefix carries the colon)
/// * prefix `cl`         -> `claude@joy`    (bash stripped `ai:`)
/// * prefix `xyz`        -> None
fn match_member(id: &str, prefix: &str) -> Option<String> {
    let id_lc = id.to_ascii_lowercase();
    let prefix_lc = prefix.to_ascii_lowercase();
    if id_lc.starts_with(&prefix_lc) {
        return Some(id.to_string());
    }
    if !prefix.contains(':') {
        if let Some((_head, tail)) = id.rsplit_once(':') {
            if tail.to_ascii_lowercase().starts_with(&prefix_lc) {
                return Some(tail.to_string());
            }
        }
    }
    None
}

/// Filter members against the prefix, returning the appropriately
/// shell-aware candidate strings in deterministic order. The optional
/// `predicate` further restricts which member IDs are considered (e.g.
/// AI-only).
fn member_candidates<'a, I, P>(members: I, prefix: &str, predicate: P) -> Vec<String>
where
    I: Iterator<Item = &'a String>,
    P: Fn(&str) -> bool,
{
    let mut out: Vec<String> = members
        .filter(|id| predicate(id.as_str()))
        .filter_map(|id| match_member(id.as_str(), prefix))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Complete AI member IDs only, used for AI-specific commands like
/// `joy auth token add` and `joy ai reset`.
pub fn complete_ai_member(current: &OsStr) -> Vec<CompletionCandidate> {
    complete_with_predicate(current, joy_core::model::project::is_ai_member)
}

/// Complete any project member ID (human or AI). Used everywhere a
/// command accepts an existing member without restriction.
pub fn complete_member(current: &OsStr) -> Vec<CompletionCandidate> {
    complete_with_predicate(current, |_| true)
}

fn complete_with_predicate(
    current: &OsStr,
    predicate: impl Fn(&str) -> bool,
) -> Vec<CompletionCandidate> {
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
    member_candidates(project.members.keys(), prefix, predicate)
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

    fn ai_only(id: &str) -> bool {
        joy_core::model::project::is_ai_member(id)
    }

    #[test]
    fn member_candidates_ai_only() {
        let members: Vec<String> = vec![
            "horst@joydev.com".into(),
            "ai:claude@joy".into(),
            "ai:qwen@joy".into(),
            "alice@team.com".into(),
        ];
        let out = member_candidates(members.iter(), "", ai_only);
        assert_eq!(out, vec!["ai:claude@joy", "ai:qwen@joy"]);
    }

    #[test]
    fn member_candidates_includes_humans_when_unfiltered() {
        let members: Vec<String> = vec![
            "alice@team.com".into(),
            "ai:claude@joy".into(),
            "bob@team.com".into(),
        ];
        let out = member_candidates(members.iter(), "", |_| true);
        assert_eq!(out, vec!["ai:claude@joy", "alice@team.com", "bob@team.com"]);
    }

    #[test]
    fn member_candidates_respects_prefix() {
        let members: Vec<String> = vec![
            "ai:claude@joy".into(),
            "ai:qwen@joy".into(),
            "ai:copilot@joy".into(),
        ];
        let out = member_candidates(members.iter(), "ai:c", ai_only);
        assert_eq!(out, vec!["ai:claude@joy", "ai:copilot@joy"]);
    }

    #[test]
    fn member_candidates_empty_on_no_match() {
        let members: Vec<String> = vec!["ai:claude@joy".into()];
        let out = member_candidates(members.iter(), "ai:x", ai_only);
        assert!(out.is_empty());
    }

    #[test]
    fn match_member_full_prefix() {
        assert_eq!(
            match_member("ai:claude@joy", "ai"),
            Some("ai:claude@joy".to_string())
        );
        assert_eq!(
            match_member("ai:claude@joy", "ai:cl"),
            Some("ai:claude@joy".to_string())
        );
        assert_eq!(
            match_member("ai:claude@joy", "ai:claude@joy"),
            Some("ai:claude@joy".to_string())
        );
    }

    #[test]
    fn match_member_post_colon_prefix() {
        // bash's COMP_WORDBREAKS strips up to the last colon: the prefix
        // arrives without the `ai:` part. The candidate should be the
        // suffix only so the shell appends correctly.
        assert_eq!(
            match_member("ai:claude@joy", "cl"),
            Some("claude@joy".to_string())
        );
        assert_eq!(
            match_member("ai:claude@joy", "claude"),
            Some("claude@joy".to_string())
        );
    }

    #[test]
    fn match_member_no_match() {
        assert_eq!(match_member("ai:claude@joy", "xyz"), None);
        assert_eq!(match_member("alice@team.com", "bob"), None);
    }

    #[test]
    fn match_member_human_id() {
        assert_eq!(
            match_member("alice@team.com", "ali"),
            Some("alice@team.com".to_string())
        );
    }

    #[test]
    fn match_member_is_case_insensitive_on_full_prefix() {
        assert_eq!(
            match_member("Peter.Schmidt@joydev.com", "p"),
            Some("Peter.Schmidt@joydev.com".to_string())
        );
        assert_eq!(
            match_member("Peter.Schmidt@joydev.com", "PETER"),
            Some("Peter.Schmidt@joydev.com".to_string())
        );
    }

    #[test]
    fn match_member_is_case_insensitive_on_post_colon_branch() {
        // bash strip leaves prefix as 'CL'; the candidate id has lowercase
        // 'claude'. Match should still hit and return the suffix.
        assert_eq!(
            match_member("ai:claude@joy", "CL"),
            Some("claude@joy".to_string())
        );
        assert_eq!(
            match_member("ai:Claude@joy", "cl"),
            Some("Claude@joy".to_string())
        );
    }

    #[test]
    fn item_match_acronym_prefix() {
        assert!(matches_item_id("JOY-0001", "J"));
        assert!(matches_item_id("JI-0042", "J"));
        assert!(matches_item_id("JOY-0001", "JOY"));
        assert!(matches_item_id("JOY-0001", "joy"));
        assert!(matches_item_id("JOY-0001", "JOY-00"));
    }

    #[test]
    fn item_match_numeric_prefix() {
        // '00' should match the four-hex section regardless of acronym.
        assert!(matches_item_id("JOY-0001", "00"));
        assert!(matches_item_id("JOY-0042", "00"));
        assert!(matches_item_id("JI-0010", "00"));
        // a deeper numeric prefix narrows
        assert!(matches_item_id("JOY-0010", "0010"));
        assert!(!matches_item_id("JOY-0001", "0010"));
    }

    #[test]
    fn item_match_hex_chunk_case_insensitive() {
        // The shard suffix or the hex portion should match either case.
        assert!(matches_item_id("JOY-00AA-1B", "AA"));
        assert!(matches_item_id("JOY-00AA-1B", "aa"));
        assert!(matches_item_id("JOY-00AA-1B", "1B"));
        assert!(matches_item_id("JOY-00AA-1B", "1b"));
    }

    #[test]
    fn item_match_milestone_prefix() {
        assert!(matches_item_id("JOY-MS-01", "MS"));
        assert!(matches_item_id("JOY-MS-01", "ms"));
        assert!(matches_item_id("JOY-MS-01", "MS-01"));
    }

    #[test]
    fn item_match_no_match_returns_false() {
        assert!(!matches_item_id("JOY-0001", "FOO"));
        assert!(!matches_item_id("JI-0001", "JOX"));
    }

    #[test]
    fn item_match_empty_prefix_matches_all() {
        assert!(matches_item_id("JOY-0001", ""));
        assert!(matches_item_id("JI-MS-01", ""));
    }

    #[test]
    fn rewrite_preserves_canonical_when_case_matches() {
        assert_eq!(
            rewrite_candidate("JOY-013F-33", "13F").as_deref(),
            Some("JOY-013F-33")
        );
        assert_eq!(
            rewrite_candidate("JOY-0001", "JOY").as_deref(),
            Some("JOY-0001")
        );
    }

    #[test]
    fn rewrite_overwrites_match_with_user_case() {
        // The user-typed substring is spliced into the canonical id at the
        // matched position, so a case-sensitive shell substring filter still
        // accepts the candidate. Joy commands resolve case-insensitively.
        assert_eq!(
            rewrite_candidate("JOY-013F-33", "13f").as_deref(),
            Some("JOY-013f-33")
        );
        assert_eq!(
            rewrite_candidate("JOY-00AA-1B", "aa").as_deref(),
            Some("JOY-00aa-1B")
        );
        assert_eq!(
            rewrite_candidate("JOY-00AA-1B", "1b").as_deref(),
            Some("JOY-00AA-1b")
        );
    }

    #[test]
    fn rewrite_overwrites_full_id_prefix_with_user_case() {
        assert_eq!(
            rewrite_candidate("JOY-0001", "joy").as_deref(),
            Some("joy-0001")
        );
        assert_eq!(
            rewrite_candidate("JOY-0001", "joy-00").as_deref(),
            Some("joy-0001")
        );
    }

    #[test]
    fn rewrite_returns_none_for_no_match() {
        assert!(rewrite_candidate("JOY-0001", "FOO").is_none());
        assert!(rewrite_candidate("JI-0001", "JOX").is_none());
    }
}
