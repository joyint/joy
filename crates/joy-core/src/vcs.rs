// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! VCS abstraction layer (see ADR-010, ADR-017).
//! All version control operations go through the `Vcs` trait.
//! Currently only Git is implemented, via CLI process calls.

use std::path::Path;
use std::process::Command;

use crate::error::JoyError;

const MIN_GIT_MAJOR: u32 = 2;

/// Hosting platform type, detected from git remote URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Forge {
    GitHub,
    GitLab,
    Gitea,
    Unknown,
}

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
}

/// Git implementation of the VCS trait.
pub struct GitVcs;

/// Run a git command and return stdout as a trimmed string.
/// Returns a descriptive error if git is not found or the command fails.
fn git_output(root: &Path, args: &[&str]) -> Result<String, JoyError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                JoyError::Git("git is not installed or not in PATH".into())
            } else {
                JoyError::Git(format!("failed to run git {}: {e}", args.join(" ")))
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let cmd = format!("git {}", args.join(" "));
        return Err(JoyError::Git(if stderr.is_empty() {
            format!("{cmd} failed (exit {})", output.status.code().unwrap_or(-1))
        } else {
            format!("{cmd} failed: {stderr}")
        }));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Run a git command silently (ignore stdout/stderr), return Ok/Err.
fn git_run(root: &Path, args: &[&str]) -> Result<(), JoyError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                JoyError::Git("git is not installed or not in PATH".into())
            } else {
                JoyError::Git(format!("failed to run git {}: {e}", args.join(" ")))
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let cmd = format!("git {}", args.join(" "));
        return Err(JoyError::Git(if stderr.is_empty() {
            format!("{cmd} failed (exit {})", output.status.code().unwrap_or(-1))
        } else {
            format!("{cmd} failed: {stderr}")
        }));
    }

    Ok(())
}

impl Vcs for GitVcs {
    fn is_repo(&self, root: &Path) -> bool {
        Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(root)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    fn init_repo(&self, root: &Path) -> Result<(), JoyError> {
        git_run(root, &["init"])
    }

    fn user_email(&self) -> Result<String, JoyError> {
        let email = git_output(Path::new("."), &["config", "user.email"])?;
        if email.is_empty() {
            return Err(JoyError::Git("git user.email is empty".into()));
        }
        Ok(email)
    }

    fn version_tags(&self, root: &Path) -> Result<Vec<String>, JoyError> {
        let output = git_output(root, &["tag", "--list", "--sort=-v:refname"]).unwrap_or_default();

        let tags: Vec<String> = output
            .lines()
            .filter(|l| l.starts_with('v') || l.starts_with('V'))
            .map(|l| l.to_string())
            .collect();

        Ok(tags)
    }

    fn latest_version_tag(&self, root: &Path) -> Result<Option<String>, JoyError> {
        match git_output(root, &["describe", "--tags", "--abbrev=0", "--match", "v*"]) {
            Ok(tag) if !tag.is_empty() => Ok(Some(tag)),
            _ => Ok(None),
        }
    }
}

// -- Git config operations --

impl GitVcs {
    /// Get a git config value (local scope).
    pub fn config_get(&self, root: &Path, key: &str) -> Result<String, JoyError> {
        git_output(root, &["config", "--local", key])
    }

    /// Set a git config value (local scope).
    pub fn config_set(&self, root: &Path, key: &str, value: &str) -> Result<(), JoyError> {
        git_run(root, &["config", "--local", key, value])
    }
}

// -- Git version check --

/// Parsed git version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub raw: String,
}

impl GitVcs {
    /// Get the installed git version. Returns error if git is not found.
    pub fn version(&self) -> Result<GitVersion, JoyError> {
        let raw = git_output(Path::new("."), &["--version"])?;
        parse_git_version(&raw)
    }

    /// Check that git meets the minimum version requirement.
    pub fn check_version(&self) -> Result<GitVersion, JoyError> {
        let v = self.version()?;
        if v.major < MIN_GIT_MAJOR {
            return Err(JoyError::Git(format!(
                "git {}.{}.{} is too old (minimum: {MIN_GIT_MAJOR}.0)\n  \
                 = help: update git to version {MIN_GIT_MAJOR}.0 or newer",
                v.major, v.minor, v.patch
            )));
        }
        Ok(v)
    }
}

fn parse_git_version(raw: &str) -> Result<GitVersion, JoyError> {
    // "git version 2.43.0" or "git version 2.43.0.windows.1"
    let version_str = raw.strip_prefix("git version ").unwrap_or(raw).trim();

    let parts: Vec<&str> = version_str.splitn(4, '.').collect();
    let major: u32 = parts
        .first()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| JoyError::Git(format!("cannot parse git version: {raw}")))?;
    let minor: u32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let patch: u32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

    Ok(GitVersion {
        major,
        minor,
        patch,
        raw: raw.to_string(),
    })
}

// -- Git write operations --

impl GitVcs {
    /// Stage files for commit.
    pub fn add(&self, root: &Path, paths: &[&str]) -> Result<(), JoyError> {
        let mut args = vec!["add"];
        args.extend_from_slice(paths);
        git_run(root, &args)
    }

    /// Stage all changes (git add -A).
    pub fn add_all(&self, root: &Path) -> Result<(), JoyError> {
        git_run(root, &["add", "-A"])
    }

    /// Create a commit with a message.
    pub fn commit(&self, root: &Path, message: &str) -> Result<(), JoyError> {
        git_run(root, &["commit", "--quiet", "-m", message])
    }

    /// Create an annotated tag with a message body.
    pub fn tag_annotated(&self, root: &Path, name: &str, body: &str) -> Result<(), JoyError> {
        git_run(root, &["tag", "-a", name, "-m", body])
    }

    /// Create a lightweight tag.
    pub fn tag(&self, root: &Path, name: &str) -> Result<(), JoyError> {
        git_run(root, &["tag", name])
    }

    /// Push the current branch to a remote.
    pub fn push(&self, root: &Path, remote: &str) -> Result<(), JoyError> {
        git_run(root, &["push", "--quiet", remote])
    }

    /// Push a specific tag to a remote.
    pub fn push_tag(&self, root: &Path, remote: &str, tag: &str) -> Result<(), JoyError> {
        git_run(root, &["push", "--quiet", remote, tag])
    }

    /// Push current branch and tags in one call.
    pub fn push_with_tags(&self, root: &Path, remote: &str) -> Result<(), JoyError> {
        self.push(root, remote)?;
        git_run(root, &["push", "--quiet", remote, "--tags"])
    }

    /// Get the default remote name (usually "origin").
    pub fn default_remote(&self, root: &Path) -> Result<String, JoyError> {
        let remote = git_output(root, &["remote"])?;
        let first = remote.lines().next().unwrap_or("origin");
        Ok(first.to_string())
    }

    /// Get the remote URL for a given remote name.
    pub fn remote_url(&self, root: &Path, remote: &str) -> Result<String, JoyError> {
        git_output(root, &["remote", "get-url", remote])
    }

    /// Check if the working tree is clean.
    pub fn is_clean(&self, root: &Path) -> Result<bool, JoyError> {
        let output = git_output(root, &["status", "--porcelain"])?;
        Ok(output.is_empty())
    }

    /// Check if HEAD is exactly on a tag.
    pub fn head_is_tagged(&self, root: &Path) -> bool {
        git_output(root, &["describe", "--tags", "--exact-match", "HEAD"]).is_ok()
    }
}

// -- Forge detection --

impl GitVcs {
    /// Detect the hosting platform from the remote URL.
    pub fn detect_forge(&self, root: &Path) -> Forge {
        let remote = match self.default_remote(root) {
            Ok(r) => r,
            Err(_) => return Forge::Unknown,
        };
        let url = match self.remote_url(root, &remote) {
            Ok(u) => u,
            Err(_) => return Forge::Unknown,
        };
        parse_forge_from_url(&url)
    }
}

/// Parse forge type from a git remote URL.
pub fn parse_forge_from_url(url: &str) -> Forge {
    let lower = url.to_lowercase();
    if lower.contains("github.com") {
        Forge::GitHub
    } else if lower.contains("gitlab.com") || lower.contains("gitlab") {
        Forge::GitLab
    } else if lower.contains("gitea") || lower.contains("codeberg.org") {
        Forge::Gitea
    } else {
        Forge::Unknown
    }
}

// -- gh CLI check --

/// Check if the GitHub CLI (gh) is installed and return its version.
pub fn gh_version() -> Result<String, JoyError> {
    let output = Command::new("gh").arg("--version").output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            JoyError::Git("gh (GitHub CLI) is not installed or not in PATH".into())
        } else {
            JoyError::Git(format!("failed to run gh: {e}"))
        }
    })?;

    if !output.status.success() {
        return Err(JoyError::Git("gh --version failed".into()));
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // "gh version 2.87.3 (2026-02-24)" -> extract "2.87.3"
    let version = raw
        .lines()
        .next()
        .unwrap_or(&raw)
        .strip_prefix("gh version ")
        .unwrap_or(&raw)
        .split_whitespace()
        .next()
        .unwrap_or(&raw)
        .to_string();
    Ok(version)
}

/// Check if gh CLI is available (returns false if not installed).
pub fn has_gh() -> bool {
    Command::new("gh")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Create a GitHub release using the gh CLI.
pub fn gh_create_release(
    root: &Path,
    tag: &str,
    title: &str,
    notes: &str,
) -> Result<String, JoyError> {
    let output = Command::new("gh")
        .args(["release", "create", tag, "--title", title, "--notes", notes])
        .current_dir(root)
        .output()
        .map_err(|e| JoyError::Git(format!("failed to run gh release create: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(JoyError::Git(format!("gh release create failed: {stderr}")));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Default VCS provider. Returns the Git implementation.
pub fn default_vcs() -> GitVcs {
    GitVcs
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

    #[test]
    fn git_vcs_is_repo() {
        let vcs = GitVcs;
        assert!(vcs.is_repo(Path::new(".")));
    }

    #[test]
    #[ignore] // requires full clone with tags
    fn git_vcs_version_tags() {
        let vcs = GitVcs;
        let tags = vcs.version_tags(Path::new(".")).unwrap();
        assert!(!tags.is_empty());
    }

    #[test]
    fn parse_git_version_standard() {
        let v = parse_git_version("git version 2.43.0").unwrap();
        assert_eq!(v.major, 2);
        assert_eq!(v.minor, 43);
        assert_eq!(v.patch, 0);
    }

    #[test]
    fn parse_git_version_windows() {
        let v = parse_git_version("git version 2.43.0.windows.1").unwrap();
        assert_eq!(v.major, 2);
        assert_eq!(v.minor, 43);
        assert_eq!(v.patch, 0);
    }

    #[test]
    fn parse_git_version_old() {
        let v = parse_git_version("git version 1.8.5").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 8);
        assert_eq!(v.patch, 5);
    }

    #[test]
    fn forge_detection_github() {
        assert_eq!(
            parse_forge_from_url("git@github.com:joyint/joy.git"),
            Forge::GitHub
        );
        assert_eq!(
            parse_forge_from_url("https://github.com/joyint/joy.git"),
            Forge::GitHub
        );
    }

    #[test]
    fn forge_detection_gitlab() {
        assert_eq!(
            parse_forge_from_url("git@gitlab.com:user/repo.git"),
            Forge::GitLab
        );
        assert_eq!(
            parse_forge_from_url("https://gitlab.example.com/user/repo.git"),
            Forge::GitLab
        );
    }

    #[test]
    fn forge_detection_gitea() {
        assert_eq!(
            parse_forge_from_url("https://codeberg.org/user/repo.git"),
            Forge::Gitea
        );
        assert_eq!(
            parse_forge_from_url("https://gitea.example.com/user/repo.git"),
            Forge::Gitea
        );
    }

    #[test]
    fn forge_detection_unknown() {
        assert_eq!(
            parse_forge_from_url("https://example.com/repo.git"),
            Forge::Unknown
        );
    }

    #[test]
    fn git_version_check() {
        let vcs = GitVcs;
        let v = vcs.check_version().unwrap();
        assert!(v.major >= MIN_GIT_MAJOR);
    }

    #[test]
    fn git_clean_check() {
        let vcs = GitVcs;
        // Should not error, just return true or false
        let _ = vcs.is_clean(Path::new("."));
    }

    #[test]
    fn git_detect_forge() {
        let vcs = GitVcs;
        let forge = vcs.detect_forge(Path::new("."));
        // We're on GitHub
        assert_eq!(forge, Forge::GitHub);
    }
}
