// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Automatic git operations triggered by Joy file writes.
//! All operations are best-effort: failures print a warning but never
//! abort the Joy command.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::error::JoyError;
use crate::model::config::AutoGit;
use crate::model::project::Project;
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
        return;
    }
    remember_joy_paths(root, &kept);
}

/// The paths joy itself staged in this process, per project root (D3.4).
/// [`auto_git_post_command`] commits these and nothing else, so a commit
/// joy writes carries joy's own writes and never a person's half staged
/// work. Process state on purpose: it is the record of what THIS run did,
/// and it must not outlive the run or travel to another checkout.
static JOY_STAGED: OnceLock<Mutex<HashMap<PathBuf, BTreeSet<String>>>> = OnceLock::new();

fn joy_staged() -> &'static Mutex<HashMap<PathBuf, BTreeSet<String>>> {
    JOY_STAGED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record paths joy staged in `root`, normalised the way an index entry
/// spells them (forward slashes, no leading `./`).
fn remember_joy_paths(root: &Path, paths: &[&str]) {
    let Ok(mut staged) = joy_staged().lock() else {
        return;
    };
    let entry = staged.entry(root.to_path_buf()).or_default();
    for path in paths {
        let path = path.replace('\\', "/");
        let path = path.trim_start_matches("./").trim_matches('/');
        if !path.is_empty() {
            entry.insert(path.to_string());
        }
    }
}

/// What the next commit in `root` may touch: joy's own directory, which
/// is joy's by definition, plus every path this run staged (SECURITY.md,
/// `.gitignore`, `.gitattributes`, a version file a release bumped).
/// Taken out of the record, so a second command in the same process does
/// not re-commit the first one's scope.
fn take_joy_paths(root: &Path) -> Vec<String> {
    let mut paths = BTreeSet::from([store::JOY_DIR.to_string()]);
    if let Ok(mut staged) = joy_staged().lock() {
        if let Some(own) = staged.remove(root) {
            paths.extend(own);
        }
    }
    paths.into_iter().collect()
}

/// Put a scope back after a commit that did not happen, so the paths are
/// not lost for the next write in the same run.
fn return_joy_paths(root: &Path, paths: Vec<String>) {
    if let Ok(mut staged) = joy_staged().lock() {
        staged.entry(root.to_path_buf()).or_default().extend(paths);
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
    let project = store::load_project(root).ok();

    let message = format!(
        "joy: {summary}\n\nCo-Authored-By: {}",
        at_rest_identity(identity, project.as_ref())
    );
    warn_about_a_missing_item(&message, project.as_ref());
    let signature = match acting_signature(root, identity, project.as_ref()) {
        Ok(signature) => signature,
        Err(e) => {
            eprintln!("Warning: auto-git commit skipped: {e}");
            return;
        }
    };
    let paths = take_joy_paths(root);
    match crate::vcs::forge::commit_index_paths(root, &paths, &message, &signature.0, &signature.1)
    {
        // nothing under joy's paths that HEAD does not already carry
        Ok(None) => return,
        Ok(Some(_)) => {}
        Err(e) => {
            eprintln!("Warning: auto-git commit failed: {e}");
            return_joy_paths(root, paths);
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
fn acting_signature(
    root: &Path,
    identity: &str,
    project: Option<&Project>,
) -> Result<(String, String), JoyError> {
    let member = identity.split_whitespace().next().unwrap_or_default();
    if !member.is_empty() {
        return crate::identity::commit_signature(root, member);
    }
    let project = project.ok_or(JoyError::UnknownActingMember)?;
    let member = crate::identity::acting_member(root, project, None)?;
    crate::identity::commit_signature(root, &member)
}

/// The acting member as this project writes it down (ADR-042), for the
/// trailer of a commit message.
///
/// `identity` is usually already at rest, because
/// [`crate::identity::Identity::log_user`] writes the member's id. The
/// auth and crypt paths hand over a raw address instead (J11 moves them),
/// and in an anonymous project that address may not be committed, in the
/// signature nor in the message. A member the project cannot resolve is
/// left as it is: the signature gate refuses such a member before any
/// commit happens, so the fallthrough is unreachable where it would
/// matter.
fn at_rest_identity(identity: &str, project: Option<&Project>) -> String {
    let Some(project) = project else {
        return identity.to_string();
    };
    let mut words = identity.split_whitespace();
    let Some(member) = words.next() else {
        return identity.to_string();
    };
    let key = project
        .member_by_key(member)
        .is_some()
        .then(|| member.to_string())
        .or_else(|| crate::privacy::member_key_for_email(project, member))
        .unwrap_or_else(|| member.to_string());
    let rest: Vec<&str> = words.collect();
    if rest.is_empty() {
        key
    } else {
        format!("{key} {}", rest.join(" "))
    }
}

/// The item reference rule of D3.3, applied to joy's own commit.
///
/// libgit2 runs no hooks, so `.joy/hooks/commit-msg` never sees the
/// commits joy writes for itself and the rule it enforces for a person's
/// `git commit` would be enforced for nobody here. It WARNS and proceeds,
/// as D3.3 decides: refusing would strand the write joy just made with
/// uncommitted `.joy` changes and no way for the person to fix a message
/// they never typed.
fn warn_about_a_missing_item(message: &str, project: Option<&Project>) {
    let Some(acronym) = project.and_then(|p| p.acronym.as_deref()) else {
        return;
    };
    if let Err(e) = crate::commit_msg::validate(message, acronym) {
        // One line, not the hook's full diagnostic: the person did not
        // write this message and cannot fix it, and the commit happened
        // anyway. The diagnostic belongs to the paths that refuse.
        eprintln!(
            "Warning: the commit joy just wrote references no {} item: {}",
            e.acronym, e.subject
        );
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
