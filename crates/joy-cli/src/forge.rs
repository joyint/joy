// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Forge abstraction: create releases on hosting platforms.
//! See ADR-017 and docs/dev/vision/ForgeSync.md.
#![allow(dead_code)] // Used by the --full release flow (JOY-0043)

use std::path::Path;

use anyhow::{bail, Result};

use joy_core::vcs;

/// Trait for hosting platform operations.
pub trait ForgeRelease {
    /// Create a release on the forge. Returns the release URL if available.
    fn create_release(
        &self,
        root: &Path,
        tag: &str,
        title: &str,
        notes: &str,
    ) -> Result<Option<String>>;
}

/// GitHub implementation using the gh CLI.
struct GitHubForge;

impl ForgeRelease for GitHubForge {
    fn create_release(
        &self,
        root: &Path,
        tag: &str,
        title: &str,
        notes: &str,
    ) -> Result<Option<String>> {
        // Check gh is available
        let gh_ver = vcs::gh_version().map_err(|_| {
            anyhow::anyhow!(
                "forge 'github' requires gh CLI\n  \
                 = help: install gh (https://cli.github.com) or run `joy forge setup` to reconfigure"
            )
        })?;

        // Check gh is authenticated
        let auth_status = std::process::Command::new("gh")
            .args(["auth", "status"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        if !auth_status.is_ok_and(|s| s.success()) {
            bail!(
                "gh is installed ({gh_ver}) but not authenticated\n  \
                 = help: run `gh auth login` or `joy forge setup`"
            );
        }

        // Idempotent: a release may already exist if an earlier
        // `just publish` made it and the subsequent push failed, or
        // if the forge workflow created it from a tag push. In both
        // cases the release is already there and we should surface
        // its URL instead of erroring out.
        let existing = std::process::Command::new("gh")
            .args(["release", "view", tag, "--json", "url", "--jq", ".url"])
            .current_dir(root)
            .output();
        if let Ok(output) = existing {
            if output.status.success() {
                let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !url.is_empty() {
                    return Ok(Some(url));
                }
            }
        }

        let url = vcs::gh_create_release(root, tag, title, notes)?;
        Ok(Some(url))
    }
}

/// No-op forge for `forge: none` or unset.
struct NoForge;

impl ForgeRelease for NoForge {
    fn create_release(
        &self,
        _root: &Path,
        _tag: &str,
        _title: &str,
        _notes: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }
}

/// Parse the forge: config value and return the appropriate implementation.
/// Returns None if forge is not set (treated as none).
pub fn from_config(forge_value: Option<&str>) -> Box<dyn ForgeRelease> {
    match forge_value {
        Some("github") => Box::new(GitHubForge),
        Some("none") | None => Box::new(NoForge),
        Some(other) if other.contains("@joyint") => {
            // github@joyint, gitlab@joyint etc. -- Joyint API not yet implemented
            eprintln!(
                "warning: forge '{other}' uses Joyint as host (not yet supported for releases)"
            );
            eprintln!("  = note: falling back to git tag only");
            Box::new(NoForge)
        }
        Some(other) => {
            eprintln!("warning: forge '{other}' is not yet supported for releases");
            eprintln!("  = note: falling back to git tag only");
            Box::new(NoForge)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_forge_returns_none() {
        let forge = from_config(None);
        let result = forge
            .create_release(Path::new("."), "v0.1.0", "test", "notes")
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn explicit_none_returns_none() {
        let forge = from_config(Some("none"));
        let result = forge
            .create_release(Path::new("."), "v0.1.0", "test", "notes")
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn unsupported_forge_warns() {
        let forge = from_config(Some("gitlab"));
        let result = forge
            .create_release(Path::new("."), "v0.1.0", "test", "notes")
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn joyint_forge_warns() {
        let forge = from_config(Some("github@joyint"));
        let result = forge
            .create_release(Path::new("."), "v0.1.0", "test", "notes")
            .unwrap();
        assert!(result.is_none());
    }
}
