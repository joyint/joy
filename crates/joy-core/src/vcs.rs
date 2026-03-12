// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! VCS abstraction layer (see ADR-010).
//! All version control operations go through the `Vcs` trait.
//! Currently only Git is implemented.

use std::path::Path;
use std::process::Command;

use crate::error::JoyError;

/// VCS operations that Joy needs.
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
        let status = Command::new("git")
            .arg("init")
            .current_dir(root)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| JoyError::Git(format!("failed to run git init: {e}")))?;

        if !status.success() {
            return Err(JoyError::Git("git init failed".into()));
        }
        Ok(())
    }

    fn user_email(&self) -> Result<String, JoyError> {
        let output = Command::new("git")
            .args(["config", "user.email"])
            .output()
            .map_err(|_| JoyError::Git("failed to run git config".to_string()))?;

        if !output.status.success() {
            return Err(JoyError::Git("git user.email not configured".to_string()));
        }

        let email = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if email.is_empty() {
            return Err(JoyError::Git("git user.email is empty".to_string()));
        }
        Ok(email)
    }

    fn version_tags(&self, root: &Path) -> Result<Vec<String>, JoyError> {
        let output = Command::new("git")
            .args(["tag", "--list", "--sort=-v:refname"])
            .current_dir(root)
            .output()
            .map_err(|e| JoyError::Git(format!("failed to run git tag: {e}")))?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let tags: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| l.starts_with('v') || l.starts_with('V'))
            .map(|l| l.to_string())
            .collect();

        Ok(tags)
    }

    fn latest_version_tag(&self, root: &Path) -> Result<Option<String>, JoyError> {
        let output = Command::new("git")
            .args(["describe", "--tags", "--abbrev=0", "--match", "v*"])
            .current_dir(root)
            .output()
            .map_err(|e| JoyError::Git(format!("failed to run git describe: {e}")))?;

        if !output.status.success() {
            return Ok(None);
        }

        let tag = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if tag.is_empty() {
            Ok(None)
        } else {
            Ok(Some(tag))
        }
    }
}

/// Default VCS provider. Returns the Git implementation.
pub fn default_vcs() -> GitVcs {
    GitVcs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // requires git user.email configured (not available in CI)
    fn git_vcs_user_email() {
        let vcs = GitVcs;
        let result = vcs.user_email();
        assert!(result.is_ok());
        assert!(!result.unwrap().is_empty());
    }

    #[test]
    fn git_vcs_is_repo() {
        let vcs = GitVcs;
        // We are running inside the joy repo
        assert!(vcs.is_repo(Path::new(".")));
    }

    #[test]
    #[ignore] // requires full clone with tags (CI uses shallow clone)
    fn git_vcs_version_tags() {
        let vcs = GitVcs;
        let tags = vcs.version_tags(Path::new(".")).unwrap();
        assert!(!tags.is_empty());
    }
}
