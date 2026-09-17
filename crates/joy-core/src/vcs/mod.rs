// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! VCS abstraction layer (see ADR-010, ADR-017).
//! All version control operations go through the `Vcs` trait.
//! Currently only Git is implemented.
//!
//! Since JOY-01FD-ED (design D3.2) there is ONE git engine and it is
//! git2: no verb in this module, and nothing it calls, starts a git
//! process. The reason is the operator's, recorded on that item and
//! mobile: the app must work on a machine that has no git binary at
//! all, and a layer that is "mostly git2" is a layer that fails there
//! on the one path nobody tested.
//!
//! What that costs is written down rather than discovered: libgit2 runs
//! no hooks and no clean or smudge filter, and it cannot sign (D3.6).
//! The hook rule joy's own commits would have missed is enforced in
//! process instead ([`crate::commit_msg::validate`], D3.3), and the
//! commit paths are scoped to the paths joy wrote so a filtered path in
//! a person's checkout is never one of them (D3.4).

use std::path::Path;

use crate::error::JoyError;

/// VCS read operations that Joy needs.
pub trait Vcs {
    /// Check if the given directory is inside a VCS repository.
    fn is_repo(&self, root: &Path) -> bool;

    /// Initialize a new repository at the given path.
    fn init_repo(&self, root: &Path) -> Result<(), JoyError>;

    /// Get the current user's email from VCS config.
    fn user_email(&self) -> Result<String, JoyError>;

    /// List all version tags (e.g. v0.5.0), sorted descending.
    fn version_tags(&self, root: &Path) -> Result<Vec<String>, JoyError>;

    /// Get the latest reachable version tag, if any.
    fn latest_version_tag(&self, root: &Path) -> Result<Option<String>, JoyError>;

    /// Read a per-clone VCS-config value. The semantics are
    /// Git-flavoured (`git config --local <key>`); other backends
    /// implement on the closest equivalent (jj has `jj config`, pijul
    /// `pijul config`).
    fn config_get(&self, root: &Path, key: &str) -> Result<String, JoyError>;

    /// Write a per-clone VCS-config value. Same semantics as
    /// [`Vcs::config_get`].
    fn config_set(&self, root: &Path, key: &str, value: &str) -> Result<(), JoyError>;
}

/// Git implementation of the VCS trait.
pub struct GitVcs;

/// How this host contacts a forge from a person's or an agent's
/// checkout: the machine's own credentials, for the host kind the entry
/// point of this process decided (D1.1). The engine's prompt rule of
/// D1.10 hangs off that word, so a hook, a worker and an agent under
/// `JOY_SESSION` are never asked anything.
fn local_auth() -> forge::Auth {
    forge::Auth::LocalAs(crate::host::process_host())
}

impl Vcs for GitVcs {
    fn is_repo(&self, root: &Path) -> bool {
        forge::is_worktree(root)
    }

    fn init_repo(&self, root: &Path) -> Result<(), JoyError> {
        forge::init_worktree(root).map_err(|e| JoyError::Git(format!("git init failed: {e}")))
    }

    fn user_email(&self) -> Result<String, JoyError> {
        forge::user_email().ok_or_else(|| JoyError::Git("git user.email is empty".into()))
    }

    fn version_tags(&self, root: &Path) -> Result<Vec<String>, JoyError> {
        Ok(forge::version_tags(root))
    }

    fn latest_version_tag(&self, root: &Path) -> Result<Option<String>, JoyError> {
        Ok(forge::describe_version_tag(root))
    }

    fn config_get(&self, root: &Path, key: &str) -> Result<String, JoyError> {
        forge::local_config_get(root, key)
            .ok_or_else(|| JoyError::Git(format!("git config --local {key} is not set")))
    }

    fn config_set(&self, root: &Path, key: &str, value: &str) -> Result<(), JoyError> {
        forge::local_config_set(root, key, value)
            .map_err(|e| JoyError::Git(format!("git config --local {key} failed: {e}")))
    }
}

// -- The git engine this build carries --

/// The version of the git engine joy runs on.
///
/// It used to be the version of the git BINARY on PATH, read from
/// `git --version`. There is no such binary in joy's path any more, and
/// the version that decides what joy can do is the library's, so this is
/// libgit2's (D3.2). A machine with no git installed reports the same
/// version as a machine with git 2.51, because for joy it is the same
/// machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub raw: String,
}

/// The oldest libgit2 whose behaviour joy is written against. The
/// vendored library is compiled into this binary, so this can only fail
/// for a build that linked a system libgit2 on purpose.
const MIN_ENGINE_MAJOR: u32 = 1;

impl GitVcs {
    /// The git engine's version. Infallible in practice (the library is
    /// linked in), and fallible in signature so the callers that check
    /// it keep reading the same way.
    pub fn version(&self) -> Result<GitVersion, JoyError> {
        let (major, minor, patch) = git2::Version::get().libgit2_version();
        Ok(GitVersion {
            major,
            minor,
            patch,
            raw: format!("libgit2 {major}.{minor}.{patch}"),
        })
    }

    /// Check that the engine meets the minimum version joy is written
    /// against.
    pub fn check_version(&self) -> Result<GitVersion, JoyError> {
        let v = self.version()?;
        if v.major < MIN_ENGINE_MAJOR {
            return Err(JoyError::Git(format!(
                "the git engine is {} and joy needs libgit2 {MIN_ENGINE_MAJOR}.0 or newer",
                v.raw
            )));
        }
        Ok(v)
    }
}

// -- Git write operations --

impl GitVcs {
    /// Stage files for commit.
    pub fn add(&self, root: &Path, paths: &[&str]) -> Result<(), JoyError> {
        forge::stage_paths(root, paths).map_err(|e| JoyError::Git(format!("git add failed: {e}")))
    }

    /// True if `path` matches one of the .gitignore patterns
    /// (regardless of whether it is currently tracked). Errors are
    /// treated as "not ignored" so that genuine staging attempts surface
    /// via the regular `add` error path rather than getting swallowed
    /// here.
    pub fn is_ignored(&self, root: &Path, path: &str) -> bool {
        forge::is_ignored(root, path)
    }

    /// Stage all changes (what `git add -A` did).
    ///
    /// In a PERSON's checkout the scoped [`GitVcs::add`] is the right
    /// verb (D3.4): this one takes whatever else was lying around with
    /// it, and no pre-commit hook stands in the way any more. What it
    /// will NOT do is write a path an external content filter governs:
    /// libgit2 runs no filter program, so a changed `filter=lfs` asset
    /// would be staged as its own bytes where the pointer belongs, and
    /// [`forge::stage_all`] refuses such a path by name instead
    /// (D3.4). The desktop's release record is the caller that made
    /// this necessary here and not only in the sweeping commit verbs.
    pub fn add_all(&self, root: &Path) -> Result<(), JoyError> {
        forge::stage_all(root).map_err(|e| JoyError::Git(format!("git add -A failed: {e}")))
    }

    /// Create a commit with a message, signed for the member this
    /// project says is acting (D4.5): libgit2 asks who commits, and the
    /// answer is joy's identity resolution and not `git config`, so a
    /// project founded without one can still be committed to.
    ///
    /// It commits the WHOLE index, so like [`GitVcs::add_all`] it is a
    /// verb for a host that staged what it wanted; the scoped path a
    /// person's checkout wants is
    /// [`forge::commit_index_paths`](forge::commit_index_paths), which
    /// `joy release record` and the auto-git commit both take. A path
    /// an external content filter governs is refused by name here too,
    /// whoever staged it (D3.4).
    pub fn commit(&self, root: &Path, message: &str) -> Result<(), JoyError> {
        let (name, email) = crate::identity::acting_signature(root)?;
        forge::commit_index(root, message, &name, &email)
            .map(|_| ())
            .map_err(|e| JoyError::Git(format!("git commit failed: {e}")))
    }

    /// Create an annotated tag with a message body.
    pub fn tag_annotated(&self, root: &Path, name: &str, body: &str) -> Result<(), JoyError> {
        let (author, email) = crate::identity::acting_signature(root)?;
        forge::tag_annotated(root, name, body, &author, &email)
            .map_err(|e| JoyError::Git(format!("git tag -a {name} failed: {e}")))
    }

    /// Create a lightweight tag.
    pub fn tag(&self, root: &Path, name: &str) -> Result<(), JoyError> {
        forge::tag_lightweight(root, name)
            .map_err(|e| JoyError::Git(format!("git tag {name} failed: {e}")))
    }

    /// Push the current branch to the forge.
    ///
    /// `remote` names what the caller believed it was pushing to; the
    /// remote really contacted is the one D1.1 picks, `origin` or else
    /// the first configured one, which is what
    /// [`GitVcs::default_remote`] answers too. The two therefore agree
    /// by construction, and the throttle key, the credential and the
    /// contacted host cannot disagree in a multi remote checkout.
    pub fn push(&self, root: &Path, remote: &str) -> Result<(), JoyError> {
        forge::push(root, &local_auth())
            .map_err(|e| contact::as_joy_error(&format!("git push {remote}"), e))
    }

    /// Push a specific tag to the forge.
    pub fn push_tag(&self, root: &Path, remote: &str, tag: &str) -> Result<(), JoyError> {
        forge::push_tag(root, &local_auth(), tag)
            .map_err(|e| contact::as_joy_error(&format!("git push {remote} {tag}"), e))
    }

    /// Push current branch and every local tag.
    pub fn push_with_tags(&self, root: &Path, remote: &str) -> Result<(), JoyError> {
        self.push(root, remote)?;
        forge::push_all_tags(root, &local_auth())
            .map_err(|e| contact::as_joy_error(&format!("git push {remote} --tags"), e))
    }

    /// The remote joy contacts for this checkout: `origin` when it is
    /// configured, otherwise the first one (D1.1).
    pub fn default_remote(&self, root: &Path) -> Result<String, JoyError> {
        forge::default_remote_name(root).ok_or_else(|| JoyError::Git("no remote configured".into()))
    }

    /// Get the remote URL for a given remote name.
    pub fn remote_url(&self, root: &Path, remote: &str) -> Result<String, JoyError> {
        forge::remotes(root)
            .into_iter()
            .find(|(name, _)| name == remote)
            .map(|(_, url)| url)
            .ok_or_else(|| JoyError::Git(format!("remote {remote} is not configured")))
    }

    /// List every configured remote as `(name, url)` pairs, in the
    /// order git2 returns them. Empty when the repo has no
    /// remotes configured.
    pub fn all_remotes(&self, root: &Path) -> Result<Vec<(String, String)>, JoyError> {
        Ok(forge::remotes(root))
    }

    /// Check if the working tree is clean (what `git status --porcelain`
    /// answered: an untracked file counts as dirty).
    pub fn is_clean(&self, root: &Path) -> Result<bool, JoyError> {
        Ok(!forge::worktree_dirty(root))
    }

    /// Check if HEAD is exactly on a tag.
    pub fn head_is_tagged(&self, root: &Path) -> bool {
        forge::head_is_tagged(root)
    }
}

/// Default VCS provider. Returns the Git implementation.
pub mod certificates;
pub mod contact;
pub mod credential_helper;
pub mod forge;
pub mod known_hosts;
pub mod maintenance;
pub mod proxy;
pub mod remote_url;
pub mod resolver;
pub mod ssh_auth;
pub mod ssh_config;

/// Who is at the other end of an operation (D1.1). The engine speaks
/// about the host kind through `vcs::HostKind`, and it is the ONE type
/// declared in [`crate::host`], where the entry point of each host sets
/// it (JOY-02A2-27).
pub use crate::host::HostKind;

pub fn default_vcs() -> GitVcs {
    GitVcs
}

// ---- named git helpers (JOY-0265-D7, now on git2) -----------------------
//
// The scattered direct `git` invocations of joy-cli and joy-ai live here
// as ONE named verb each. They used to stay on the git BINARY on
// purpose, so a person's config, credential setup and hooks applied;
// D3.2 ends that, because a machine without git has no such binary and
// the product still has to work there. What the binary did for them is
// done here instead: libgit2 reads the same config files, the
// credentials come from `vcs::resolver`, and the hooks a person
// installed are chained from joy's own (D3.5) rather than run by joy.

/// The unix time of a commit, resolved in the CURRENT directory (merge
/// drivers run with the repo as cwd). `None` when the rev does not
/// resolve or the directory is no checkout.
pub fn commit_unix_time(rev: &str) -> Option<i64> {
    let rev = rev.trim();
    // A merge driver whose %O/%A/%B was not substituted hands the
    // placeholder through; it is not a rev and must not be looked up.
    if rev.is_empty() || rev.starts_with('%') {
        return None;
    }
    forge::commit_unix_time(Path::new("."), rev)
}

/// Staged paths relative to the repo root (added/modified/renamed),
/// which is what `git diff --cached --name-only --diff-filter=ACMR`
/// answered.
pub fn staged_paths(root: &Path) -> Vec<String> {
    forge::staged_paths(root)
}

/// Whether `remote` is configured in this checkout.
///
/// git2, not a git process. Reading the remote list is local plumbing:
/// it needs no transport, so it needs neither the `forge-net` feature
/// nor the user's ambient credentials, and there is nothing a git
/// process adds. It sat on the chat write path as
/// `git -C <root> remote get-url origin`, one spawn per send and per
/// read, and the git2 only rule leaves no room for it (D3.2, D3.7).
///
/// The two answers are not identical, and the difference is deliberate:
/// `git remote get-url <name>` fails for a remote that carries only
/// `remote.<name>.pushurl`, while `git_remote_lookup` succeeds whenever
/// either `url` or `pushurl` is configured. A remote joy can push to is
/// a remote that exists, which is the question every caller here asks
/// (the chat send and read gate in joy-cli), so the git2 answer is the
/// better one. Pinned by the cases below because it is a silent change.
pub fn remote_exists(root: &Path, remote: &str) -> bool {
    git2::Repository::discover(root)
        .and_then(|repo| repo.find_remote(remote).map(|_| ()))
        .is_ok()
}

/// Whether git tracks `path` in this checkout.
pub fn path_is_tracked(root: &Path, path: &str) -> bool {
    forge::path_is_tracked(root, path)
}

/// Untrack `path` (keep the file). Warns and answers false on failure.
pub fn rm_cached(root: &Path, path: &str) -> bool {
    match forge::untrack_path(root, path) {
        Ok(gone) => gone > 0,
        Err(e) => {
            eprintln!("Warning: could not untrack {path}: {e}");
            false
        }
    }
}

/// Remove `path` from tree and index. Warns and answers false on failure.
pub fn rm_hard(root: &Path, path: &str) -> bool {
    match forge::remove_path(root, path) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("Warning: could not remove {path}: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // requires git user.email configured
    fn git_vcs_user_email() {
        let vcs = GitVcs;
        let result = vcs.user_email();
        assert!(result.is_ok());
        assert!(!result.unwrap().is_empty());
    }

    /// `remote_exists` moved from a `git remote get-url` spawn to
    /// git2, so what it answers is pinned here rather than only "no git
    /// process ran".
    #[test]
    fn remote_exists_answers_off_the_configured_remotes() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        repo.remote("origin", "https://example.invalid/a.git")
            .unwrap();

        assert!(remote_exists(dir.path(), "origin"));
        assert!(!remote_exists(dir.path(), "upstream"));

        // A remote with a push url and no fetch url exists too. The git
        // process this replaced said no here; joy asks whether there is
        // a remote to push to, and there is.
        repo.config()
            .unwrap()
            .set_str("remote.mirror.pushurl", "https://example.invalid/b.git")
            .unwrap();
        assert!(remote_exists(dir.path(), "mirror"));

        // A directory that is no checkout at all has no remotes, and the
        // answer is false rather than an error.
        let plain = tempfile::tempdir().unwrap();
        assert!(!remote_exists(plain.path(), "origin"));
    }

    #[test]
    fn git_vcs_is_repo() {
        let vcs = GitVcs;
        assert!(vcs.is_repo(Path::new(".")));
    }

    /// The version is the ENGINE's now, and it is there without a git
    /// binary anywhere near the machine.
    #[test]
    fn the_engine_reports_its_own_version() {
        let vcs = GitVcs;
        let v = vcs.check_version().unwrap();
        assert!(v.major >= MIN_ENGINE_MAJOR);
        assert!(v.raw.starts_with("libgit2 "), "{}", v.raw);
        assert_eq!(v, vcs.version().unwrap());
    }

    #[test]
    fn git_clean_check() {
        let vcs = GitVcs;
        // Should not error, just return true or false
        let _ = vcs.is_clean(Path::new("."));
    }
}
