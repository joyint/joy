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
    let kept = match default_vcs().add_unignored(root, paths) {
        Ok(kept) => kept,
        Err(e) => {
            eprintln!("Warning: auto-git add failed: {e}");
            return;
        }
    };
    remember_joy_paths(root, &kept);
}

/// The paths joy itself staged in this process, per project root.
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
/// `identity` is the Joy identity string for the commit signature and delegation, as
/// [`crate::identity::Identity::log_user`] writes it: the acting member,
/// optionally followed by `delegated-by:<human>`.
///
/// The commit is signed for that acting member, not for whatever
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

    let project = store::load_project(root).ok();

    let resolved = at_rest_identity(identity, project.as_ref());
    let message = match resolved.split_once(" delegated-by:") {
        Some((member, delegator)) if crate::model::project::is_ai_member(member) => {
            format!("joy: {summary}\n\nDelegated-By: {delegator}")
        }
        _ => format!("joy: {summary}"),
    };
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

    push_if_asked(root, level);
}

/// Push what joy just committed when `workflow.auto-git` says `push`.
fn push_if_asked(root: &Path, level: AutoGit) {
    if !level.should_push() {
        return;
    }
    let vcs = default_vcs();
    let remote = vcs.default_remote(root).unwrap_or_else(|_| "origin".into());
    if let Err(e) = vcs.push(root, &remote) {
        say_contact_aside(
            root,
            "auto-git push failed",
            "the commit stays local, the next joy write retries",
            &e,
        );
    }
}

/// What became of the changes a sync made in a checkout (JOY-02CF-EE).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCommit {
    /// One commit carries them; this is its id.
    Written(String),
    /// The sync changed nothing that joy commits.
    Nothing,
    /// `workflow.auto-git: off`: joy touches git for nobody.
    Off,
    /// They stay in the checkout, and this says why.
    Left(String),
}

/// The message of the commit a sync's changes go into: in general words
/// that the project moved from one joy version to another, under a
/// subject that names no item, because no item asked for it.
pub fn project_update_message(from: Option<&str>, to: &str, delegator: Option<&str>) -> String {
    let (subject, body) = match from {
        Some(from) if from != to => (
            format!("chore(joy): project files updated from joy {from} to joy {to} [no-item]"),
            format!(
                "joy brought the files it manages in this project from version {from} to version {to}."
            ),
        ),
        Some(_) => (
            format!("chore(joy): project files refreshed by joy {to} [no-item]"),
            format!("joy brought the files it manages in this project back in line with version {to}."),
        ),
        None => (
            format!("chore(joy): project files set up for joy {to} [no-item]"),
            format!("joy wrote the files it manages in this project for version {to}."),
        ),
    };
    match delegator {
        Some(delegator) => format!("{subject}\n\n{body}\n\nDelegated-By: {delegator}"),
        None => format!("{subject}\n\n{body}"),
    }
}

/// Commit what a sync changed, as ONE commit that carries exactly those
/// paths (JOY-02CF-EE).
///
/// `changed` is [`crate::update::changed_since`]'s answer and `identity`
/// the acting member in the event-log form, as for
/// [`auto_git_post_command`]; an empty one lets the project say who acts
/// in this checkout. Unlike a command's own commit this one does not wait
/// for `workflow.auto-git: commit`: what a sync leaves uncommitted is
/// taken for a leftover and thrown away. Only `off` holds it back, where
/// joy touches git for nobody.
pub fn commit_project_update(
    root: &Path,
    changed: &crate::update::Changed,
    identity: &str,
) -> UpdateCommit {
    commit_project_update_at(root, changed, identity, auto_git_level())
}

fn commit_project_update_at(
    root: &Path,
    changed: &crate::update::Changed,
    identity: &str,
    level: AutoGit,
) -> UpdateCommit {
    if changed.paths.is_empty() {
        return UpdateCommit::Nothing;
    }
    if level == AutoGit::Off {
        return UpdateCommit::Off;
    }
    let project = store::load_project(root).ok();
    let resolved = at_rest_identity(identity, project.as_ref());
    let delegator = match resolved.split_once(" delegated-by:") {
        Some((member, delegator)) if crate::model::project::is_ai_member(member) => Some(delegator),
        _ => None,
    };
    let message = project_update_message(
        changed.from.as_deref(),
        crate::update::CURRENT_VERSION,
        delegator,
    );
    let signature = match acting_signature(root, identity, project.as_ref()) {
        Ok(signature) => signature,
        Err(e) => return UpdateCommit::Left(e.to_string()),
    };
    let wanted: Vec<&str> = changed.paths.iter().map(String::as_str).collect();
    let staged: Vec<String> = match crate::vcs::forge::stage_unignored_paths(root, &wanted) {
        Ok(staged) => staged.into_iter().map(str::to_string).collect(),
        Err(e) => return UpdateCommit::Left(e.to_string()),
    };
    match crate::vcs::forge::commit_index_paths(root, &staged, &message, &signature.0, &signature.1)
    {
        Ok(Some(id)) => {
            push_if_asked(root, level);
            UpdateCommit::Written(id)
        }
        Ok(None) => UpdateCommit::Nothing,
        Err(e) => UpdateCommit::Left(e.to_string()),
    }
}

/// How this host says a forge refusal that is NOT the command's answer.
///
/// `root` is the checkout whose remote was contacted, `headline` says
/// what did not happen and `tail` what happens next; the error carries
/// the classifier's verdict.
pub type ContactAside = fn(root: &Path, headline: &str, tail: &str, error: &JoyError);

static CONTACT_ASIDE: OnceLock<ContactAside> = OnceLock::new();

/// Lend joy-core this host's way of saying such a refusal.
///
/// The auto-git push is a forge contact, and with `workflow.auto-git:
/// push` it runs after nearly every joy write, so its failure has to
/// speak the one vocabulary like every other contact: the state
/// word, the plain sentence, the one next step, and in `--json` mode one
/// object on stderr rather than a line of prose. The words live here
/// (`vcs::contact`), the `--json` decision and the CLI's own next steps
/// live in joy-cli, so the host installs its sink once at its entry
/// point. A host that installs nothing still says the same three
/// things, without the JSON and without a `joy` command in the help
/// line, so no path is left silent.
pub fn set_contact_aside(sink: ContactAside) {
    let _ = CONTACT_ASIDE.set(sink);
}

fn say_contact_aside(root: &Path, headline: &str, tail: &str, error: &JoyError) {
    if let Some(sink) = CONTACT_ASIDE.get() {
        sink(root, headline, tail, error);
        return;
    }
    let failure = error.failure();
    let contact = error.contact();
    // The host joy really contacted, so the sentence names no other.
    // Empty when there is none, and then it names none.
    let host = crate::vcs::forge::remote_url(root)
        .map(|url| crate::vcs::contact::host_of(&url))
        .unwrap_or_default();
    let message = contact
        .map(|c| c.message.clone())
        .unwrap_or_else(|| failure.sentence(&host));
    eprintln!("{headline}: {message}");
    eprintln!("  = note: state {}; {tail}", failure.reason());
    if let Some(action) = contact
        .and_then(|c| c.action.clone())
        .or_else(|| failure.guidance().map(str::to_string))
    {
        eprintln!("  = help: {action}");
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
/// signature and delegation trailer of a commit message.
///
/// `identity` is usually already at rest, because
/// [`crate::identity::Identity::log_user`] writes the member's id. The
/// auth and crypt paths hand over a raw address instead,
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

/// The item reference rule, applied to joy's own commit.
///
/// libgit2 runs no hooks, so `.joy/hooks/commit-msg` never sees the
/// commits joy writes for itself and the rule it enforces for a person's
/// `git commit` would be enforced for nobody here. It WARNS and proceeds,
/// as the rule decides: refusing would strand the write joy just made with
/// uncommitted `.joy` changes and no way for the person to fix a message
/// they never typed.
fn warn_about_a_missing_item(message: &str, project: Option<&Project>) {
    let Some(acronym) = project.and_then(|p| p.acronym.as_deref()) else {
        return;
    };
    crate::commit_msg::warn_unless_referenced(message, acronym);
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

    // -- the commit a sync's changes go into (JOY-02CF-EE) ---------------

    use crate::update::Changed;
    use crate::vcs::forge;

    /// A checkout with one commit, in which a sync then changed a file
    /// under `.joy`, changed SECURITY.md and removed an old file, beside
    /// a file of the person's that is none of joy's business.
    fn a_checkout_a_sync_changed() -> (tempfile::TempDir, Changed) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        forge::init_repo(root, "main").unwrap();
        std::fs::create_dir_all(root.join(".joy")).unwrap();
        for (path, content) in [
            (".joy/config.defaults.yaml", "old"),
            ("SECURITY.md", "old"),
            ("retired.md", "old"),
            ("src.rs", "committed"),
        ] {
            std::fs::write(root.join(path), content).unwrap();
        }
        forge::stage_paths(root, &[".joy", "SECURITY.md", "retired.md", "src.rs"]).unwrap();
        forge::commit_index(root, "first [no-item]", "m", "m@e.c").unwrap();

        std::fs::write(root.join("src.rs"), "half finished").unwrap();
        forge::stage_paths(root, &["src.rs"]).unwrap();
        std::fs::write(root.join(".joy/config.defaults.yaml"), "new").unwrap();
        std::fs::write(root.join("SECURITY.md"), "new").unwrap();
        std::fs::remove_file(root.join("retired.md")).unwrap();
        let changed = Changed {
            paths: vec![
                ".joy/config.defaults.yaml".into(),
                "SECURITY.md".into(),
                "retired.md".into(),
            ],
            left: vec![],
            from: Some("0.1.0".into()),
        };
        (dir, changed)
    }

    /// Id, message, author address and changed paths of the last commit.
    fn head_commit(root: &Path) -> (String, String, String, Vec<String>) {
        let repo = git2::Repository::open(root).unwrap();
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        let id = commit.id().to_string();
        let message = commit.message().unwrap_or_default().to_string();
        let author = commit.author().email().unwrap_or_default().to_string();
        let old = commit.parent(0).unwrap().tree().unwrap();
        let new = commit.tree().unwrap();
        let mut paths = Vec::new();
        {
            let diff = repo
                .diff_tree_to_tree(Some(&old), Some(&new), None)
                .unwrap();
            for delta in diff.deltas() {
                if let Some(path) = delta.new_file().path() {
                    paths.push(path.display().to_string());
                }
            }
        }
        paths.sort();
        (id, message, author, paths)
    }

    #[test]
    fn a_syncs_changes_become_one_commit_with_exactly_those_paths() {
        let (dir, changed) = a_checkout_a_sync_changed();
        let root = dir.path();

        let outcome = commit_project_update_at(root, &changed, "horst@joydev.test", AutoGit::Add);
        let (id, message, author, paths) = head_commit(root);
        assert_eq!(outcome, UpdateCommit::Written(id));
        assert_eq!(
            paths,
            [".joy/config.defaults.yaml", "SECURITY.md", "retired.md"],
            "the removed file is part of it, the person's staged file is not"
        );
        assert_eq!(author, "horst@joydev.test");
        assert_eq!(
            message,
            project_update_message(Some("0.1.0"), crate::update::CURRENT_VERSION, None)
        );
        assert!(
            forge::checkout_picture(root).contains_key("src.rs"),
            "the person's work is still theirs to commit"
        );

        // nothing of the sync is left, so a second call writes nothing
        assert_eq!(
            commit_project_update_at(root, &changed, "horst@joydev.test", AutoGit::Add),
            UpdateCommit::Nothing
        );
    }

    #[test]
    fn an_ai_members_commit_names_who_delegated() {
        let (dir, changed) = a_checkout_a_sync_changed();
        let outcome = commit_project_update_at(
            dir.path(),
            &changed,
            "claude delegated-by:horst@joydev.test",
            AutoGit::Commit,
        );
        let (id, message, author, _) = head_commit(dir.path());
        assert_eq!(outcome, UpdateCommit::Written(id));
        assert_eq!(author, "claude");
        assert!(
            message.ends_with("\n\nDelegated-By: horst@joydev.test"),
            "{message}"
        );
    }

    #[test]
    fn nothing_is_committed_where_joy_may_not_or_cannot() {
        let (dir, changed) = a_checkout_a_sync_changed();
        let root = dir.path();
        let first = git2::Repository::open(root)
            .unwrap()
            .head()
            .unwrap()
            .target()
            .unwrap();
        let head = || {
            git2::Repository::open(root)
                .unwrap()
                .head()
                .unwrap()
                .target()
                .unwrap()
        };

        // auto-git off: joy touches git for nobody
        assert_eq!(
            commit_project_update_at(root, &changed, "horst@joydev.test", AutoGit::Off),
            UpdateCommit::Off
        );
        // nobody acts here that joy could name
        assert!(matches!(
            commit_project_update_at(root, &changed, "", AutoGit::Add),
            UpdateCommit::Left(_)
        ));
        // the sync changed nothing
        assert_eq!(
            commit_project_update_at(root, &Changed::default(), "horst@joydev.test", AutoGit::Add),
            UpdateCommit::Nothing
        );
        assert_eq!(head(), first);
    }

    #[test]
    fn the_message_says_the_version_move_in_general_words() {
        let moved = project_update_message(Some("0.22.0"), "0.23.0", None);
        assert_eq!(
            moved.lines().next(),
            Some("chore(joy): project files updated from joy 0.22.0 to joy 0.23.0 [no-item]")
        );
        assert!(
            moved.contains("from version 0.22.0 to version 0.23.0"),
            "{moved}"
        );

        let never = project_update_message(None, "0.23.0", None);
        assert_eq!(
            never.lines().next(),
            Some("chore(joy): project files set up for joy 0.23.0 [no-item]")
        );
        let same = project_update_message(Some("0.23.0"), "0.23.0", None);
        assert_eq!(
            same.lines().next(),
            Some("chore(joy): project files refreshed by joy 0.23.0 [no-item]")
        );
        // and the trailer of a delegated commit closes the message
        let delegated = project_update_message(None, "0.23.0", Some("horst@joydev.test"));
        assert!(
            delegated.ends_with("Delegated-By: horst@joydev.test"),
            "{delegated}"
        );
    }
}
