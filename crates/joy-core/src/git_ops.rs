// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Automatic git operations triggered by Joy file writes.
//! All operations are best-effort: failures print a warning but never
//! abort the Joy command.

use std::path::Path;

use crate::error::JoyError;
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
///
/// Paths that match `.gitignore` are silently skipped before `git
/// add` runs. Without that filter an accidental write to an ignored
/// path -- or a stale index entry that survived an earlier
/// ordering bug -- would either pollute the index or print git's
/// "paths are ignored" warning on every subsequent run.
pub fn auto_git_add(root: &Path, paths: &[&str]) {
    let level = auto_git_level();
    if !level.should_add() || paths.is_empty() {
        return;
    }
    let vcs = default_vcs();
    let kept: Vec<&str> = paths
        .iter()
        .copied()
        .filter(|p| !vcs.is_ignored(root, p))
        .collect();
    if kept.is_empty() {
        return;
    }
    if let Err(e) = vcs.add(root, &kept) {
        eprintln!("Warning: auto-git add failed: {e}");
    }
}

/// After a mutating command completes, commit and optionally push
/// if auto-git >= Commit.
///
/// `summary` is the commit subject line (e.g. "add JOY-005D Auto-add...").
/// `identity` is the Joy identity string for Co-Authored-By, as
/// [`crate::identity::Identity::log_user`] writes it: the acting member,
/// optionally followed by `delegated-by:<human>`.
///
/// The commit is signed for that acting member (D4.5), not for whatever
/// `git config` on this machine says: joy knows who acts, the machine
/// setting is a prefill for the display name, and in an anonymous project
/// the opaque member id is the only thing that may reach a commit
/// (ADR-042). This is also why the commit itself is written through git2
/// rather than by a git process: a project founded without a git config
/// has nothing for `git commit` to sign with.
pub fn auto_git_post_command(root: &Path, summary: &str, identity: &str) {
    let level = auto_git_level();
    if !level.should_commit() {
        return;
    }

    let vcs = default_vcs();

    let message = format!("joy: {summary}\n\nCo-Authored-By: {identity}");
    let signature = match acting_signature(root, identity) {
        Ok(signature) => signature,
        Err(e) => {
            eprintln!("Warning: auto-git commit skipped: {e}");
            return;
        }
    };
    match crate::vcs::forge::commit_index_if_changed(root, &message, &signature.0, &signature.1) {
        // nothing staged that HEAD does not already carry: not an error
        Ok(None) => return,
        Ok(Some(_)) => {}
        Err(e) => {
            eprintln!("Warning: auto-git commit failed: {e}");
            return;
        }
    }

    if level.should_push() {
        let remote = vcs.default_remote(root).unwrap_or_else(|_| "origin".into());
        if let Err(e) = vcs.push(root, &remote) {
            eprintln!("Warning: auto-git push failed: {e}");
        }
    }
}

/// The two signature fields for the member `identity` names.
///
/// `identity` is the event-log form, so the acting member is its first
/// word and a `delegated-by:` note may follow it. When the caller could
/// not name anybody (no session, no pin, no git config), the project's
/// own answer is asked for before giving up, so the message a person sees
/// is the typed one and not git2's parse error.
fn acting_signature(root: &Path, identity: &str) -> Result<(String, String), JoyError> {
    let member = identity.split_whitespace().next().unwrap_or_default();
    if !member.is_empty() {
        return crate::identity::commit_signature(root, member);
    }
    let project = store::load_project(root)?;
    let member = crate::identity::acting_member(root, &project, None)?;
    crate::identity::commit_signature(root, &member)
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
