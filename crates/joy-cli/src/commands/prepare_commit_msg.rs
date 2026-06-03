// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Hidden `prepare-commit-msg` helper (JOY-01B1-FF). Invoked by Git through
//! the hook written by `joy init`. Pre-fills the commit editor with a subject
//! that references the right Joy item(s) plus, under an AI delegation, the
//! commit trailers. End users do not run this directly.
//!
//! Git calls the hook as `prepare-commit-msg <file> [source] [sha]`. We only
//! act for an ordinary commit (no source): for message/template/merge/squash/
//! commit sources the user already supplied or is amending a message, so we
//! must not clobber it. The message-building logic itself lives in
//! `joy_core::commit_msg` and is unit-tested there; this file only gathers the
//! inputs from git and the project.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use clap::Args;

use joy_core::commit_msg::{build_suggestion, Committer, Inputs, ItemRef};
use joy_core::model::item::Status;
use joy_core::{items, store};

#[derive(Args)]
#[command(hide = true, about = "Internal: prepare-commit-msg hook helper")]
pub struct PrepareCommitMsgArgs {
    /// Path to the commit message file git passes as $1.
    msg_file: String,
    /// The commit source git passes as $2 (message, template, merge, squash,
    /// commit). Empty for an ordinary commit.
    #[arg(default_value = "")]
    source: String,
}

pub fn run(args: PrepareCommitMsgArgs) -> Result<()> {
    // Only pre-fill for an ordinary commit. Any explicit source means the
    // user already has a message (or git is amending/merging) -- do not touch.
    if !args.source.is_empty() {
        return Ok(());
    }

    let cwd = std::env::current_dir()?;
    let Some(root) = store::find_project_root(&cwd) else {
        // Not a Joy project: leave the message untouched.
        return Ok(());
    };

    // If the user already typed something (e.g. `git commit` after writing a
    // message into the file some other way), keep it. A fresh commit gives a
    // file that is empty or only git's own comment lines.
    let existing = std::fs::read_to_string(&args.msg_file).unwrap_or_default();
    if existing
        .lines()
        .any(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
    {
        return Ok(());
    }

    let staged = staged_paths(&root);

    // Items whose .joy/items/*.yaml file is staged: exact id from filename.
    let all_items = items::load_items(&root).unwrap_or_default();
    let mut staged_items: Vec<ItemRef> = Vec::new();
    for path in &staged {
        if let Some(id) = item_id_from_staged_path(path) {
            if let Some(it) = all_items.iter().find(|i| i.id == id) {
                staged_items.push(to_ref(it));
            }
        }
    }

    let in_progress: Vec<ItemRef> = all_items
        .iter()
        .filter(|i| i.status == Status::InProgress)
        .map(to_ref)
        .collect();

    let changed_crates = changed_crates(&staged);

    let identity = joy_core::identity::resolve_identity(&root).ok();
    let committer = match identity {
        Some(id) => Committer {
            member: id.member,
            delegated_by: id.delegated_by,
        },
        None => Committer {
            member: String::new(),
            delegated_by: None,
        },
    };

    let suggestion = build_suggestion(&Inputs {
        staged_items,
        in_progress,
        changed_crates,
        committer,
    });

    // Prepend our suggestion above git's existing comment block so the
    // standard "# Please enter the commit message..." help stays intact.
    let combined = format!("{suggestion}{existing}");
    std::fs::write(&args.msg_file, combined)
        .with_context(|| format!("writing prepared message to {}", args.msg_file))?;

    Ok(())
}

/// Staged paths relative to the repo root (added/modified/renamed), via
/// `git diff --cached --name-only`.
fn staged_paths(root: &Path) -> Vec<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["diff", "--cached", "--name-only", "--diff-filter=ACMR"])
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// Extract a Joy item id from a staged `.joy/items/<ID>-<slug>.yaml` path.
fn item_id_from_staged_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix(".joy/items/")?;
    let name = rest.strip_suffix(".yaml")?;
    // ID is ACRONYM-XXXX or ACRONYM-XXXX-YY (ADR-027). Take the leading
    // segments up to the title slug: split on '-' and keep while parts look
    // like an id (acronym, 4-hex, optional 2-hex).
    let parts: Vec<&str> = name.split('-').collect();
    if parts.len() < 2 {
        return None;
    }
    let is_hex = |s: &str, n: usize| s.len() == n && s.chars().all(|c| c.is_ascii_hexdigit());
    // acronym + 4-hex
    if parts.len() >= 2 && is_hex(parts[1], 4) {
        // optional 2-hex third segment
        if parts.len() >= 3 && is_hex(parts[2], 2) {
            return Some(format!("{}-{}-{}", parts[0], parts[1], parts[2]));
        }
        return Some(format!("{}-{}", parts[0], parts[1]));
    }
    None
}

/// Distinct crate names among staged code files under `crates/<name>/`.
fn changed_crates(staged: &[String]) -> Vec<String> {
    let mut crates: Vec<String> = Vec::new();
    for p in staged {
        if let Some(rest) = p.strip_prefix("crates/") {
            if let Some(name) = rest.split('/').next() {
                if !name.is_empty() && !crates.iter().any(|c| c == name) {
                    crates.push(name.to_string());
                }
            }
        }
    }
    crates
}

fn to_ref(it: &joy_core::model::item::Item) -> ItemRef {
    ItemRef {
        id: it.id.clone(),
        item_type: it.item_type.clone(),
        title: it.title.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_two_part_and_three_part_ids() {
        assert_eq!(
            item_id_from_staged_path(".joy/items/JOY-0042-auth-system.yaml").as_deref(),
            Some("JOY-0042")
        );
        assert_eq!(
            item_id_from_staged_path(".joy/items/JOY-01B1-FF-prepare-commit.yaml").as_deref(),
            Some("JOY-01B1-FF")
        );
    }

    #[test]
    fn ignores_non_item_paths() {
        assert_eq!(item_id_from_staged_path("crates/joy-cli/src/main.rs"), None);
        assert_eq!(item_id_from_staged_path(".joy/logs/2026-06-03.log"), None);
    }

    #[test]
    fn changed_crates_are_distinct() {
        let staged = vec![
            "crates/joy-cli/src/main.rs".to_string(),
            "crates/joy-cli/src/commands/x.rs".to_string(),
            "crates/joy-core/src/lib.rs".to_string(),
            ".joy/items/JOY-0001-x.yaml".to_string(),
        ];
        assert_eq!(changed_crates(&staged), vec!["joy-cli", "joy-core"]);
    }
}
