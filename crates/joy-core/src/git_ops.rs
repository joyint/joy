// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Automatic git operations triggered by Joy file writes.
//! All operations are best-effort: failures print a warning but never
//! abort the Joy command.

use std::path::Path;

use crate::model::config::AutoGit;
use crate::store;
use crate::vcs::default_vcs;

/// Read the configured auto-git level.
pub fn auto_git_level() -> AutoGit {
    store::load_config().workflow.auto_git
}

/// Stage the given paths if auto-git >= Add.
/// Paths are relative to the project root.
/// Errors are printed as warnings and swallowed.
pub fn auto_git_add(root: &Path, paths: &[&str]) {
    let level = auto_git_level();
    if !level.should_add() || paths.is_empty() {
        return;
    }
    let vcs = default_vcs();
    if let Err(e) = vcs.add(root, paths) {
        eprintln!("Warning: auto-git add failed: {e}");
    }
}

/// After a mutating command completes, commit and optionally push
/// if auto-git >= Commit.
///
/// `summary` is the commit subject line (e.g. "add JOY-005D Auto-add...").
/// `identity` is the Joy identity string for Co-Authored-By.
pub fn auto_git_post_command(root: &Path, summary: &str, identity: &str) {
    let level = auto_git_level();
    if !level.should_commit() {
        return;
    }

    let vcs = default_vcs();

    let message = format!("joy: {summary}\n\nCo-Authored-By: {identity}");
    if let Err(e) = vcs.commit(root, &message) {
        let err = e.to_string();
        // "nothing to commit" is not an error worth warning about
        if !err.contains("nothing to commit") {
            eprintln!("Warning: auto-git commit failed: {e}");
        }
        return;
    }

    if level.should_push() {
        let remote = vcs.default_remote(root).unwrap_or_else(|_| "origin".into());
        if let Err(e) = vcs.push(root, &remote) {
            eprintln!("Warning: auto-git push failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_git_level_returns_default() {
        // Outside a project root, load_config returns default (Add)
        let level = auto_git_level();
        assert_eq!(level, AutoGit::Add);
    }
}
