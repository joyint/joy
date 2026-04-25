// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Render and update the project's SECURITY.md.
//!
//! Joy ships a SECURITY.md template that documents the public-by-design
//! auth schema fields (`verify_key`, `kdf_nonce`, `enrollment_verifier`,
//! `delegation_verifier`) so SOC analysts and secret scanners have a
//! canonical explanation when keyword-based detectors flag those names.
//! Per ADR-035 the template is rendered to the project root, not to
//! `.joy/`, so GitHub and similar forges show it in their Security
//! policy tab.
//!
//! The Joy block is delimited by `<!-- joy:security begin -->` and
//! `<!-- joy:security end -->`. Content outside the markers is
//! preserved across rendering.

use std::path::Path;

use crate::error::JoyError;

const SECURITY_TEMPLATE: &str = include_str!("../templates/SECURITY.md");
const BLOCK_START: &str = "<!-- joy:security begin -->";
const BLOCK_END: &str = "<!-- joy:security end -->";

/// Return the body that the Joy block should contain.
///
/// Currently the template is fully static; rendering may take parameters
/// in the future (project name, member emails) without changing this
/// signature - callers should not assume the template is static.
pub fn rendered_body() -> &'static str {
    SECURITY_TEMPLATE
}

/// Render SECURITY.md at `path`, preserving any existing user content
/// outside the Joy markers. Returns `true` if the file was created or
/// updated, `false` if it was already current.
pub fn render(path: &Path) -> Result<bool, JoyError> {
    let body = rendered_body();
    let block = format!("{BLOCK_START}\n{body}{BLOCK_END}\n");

    let new_content = if path.is_file() {
        let existing = std::fs::read_to_string(path).map_err(|e| JoyError::ReadFile {
            path: path.to_path_buf(),
            source: e,
        })?;
        merge_block(&existing, &block)
    } else {
        block.clone()
    };

    if path.is_file() {
        let existing = std::fs::read_to_string(path).map_err(|e| JoyError::ReadFile {
            path: path.to_path_buf(),
            source: e,
        })?;
        if existing == new_content {
            return Ok(false);
        }
    }

    std::fs::write(path, new_content).map_err(|e| JoyError::WriteFile {
        path: path.to_path_buf(),
        source: e,
    })?;
    Ok(true)
}

/// Inspect `path` and report whether `render` would change anything.
pub fn is_current(path: &Path) -> Result<bool, JoyError> {
    if !path.is_file() {
        return Ok(false);
    }
    let body = rendered_body();
    let block = format!("{BLOCK_START}\n{body}{BLOCK_END}\n");
    let existing = std::fs::read_to_string(path).map_err(|e| JoyError::ReadFile {
        path: path.to_path_buf(),
        source: e,
    })?;
    Ok(existing == merge_block(&existing, &block))
}

fn merge_block(existing: &str, block: &str) -> String {
    if let (Some(start), Some(end_pos)) = (existing.find(BLOCK_START), existing.find(BLOCK_END)) {
        let end = end_pos + BLOCK_END.len();
        let mut out = String::new();
        out.push_str(&existing[..start]);
        out.push_str(block.trim_end());
        // Preserve a single newline before any user content that follows.
        let tail = &existing[end..];
        if !tail.is_empty() {
            out.push('\n');
            out.push_str(tail.trim_start_matches('\n'));
        } else {
            out.push('\n');
        }
        out
    } else {
        // No existing Joy block: append after a blank line.
        let trimmed = existing.trim_end();
        if trimmed.is_empty() {
            block.to_string()
        } else {
            format!("{trimmed}\n\n{block}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn render_creates_file_when_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("SECURITY.md");
        let changed = render(&path).unwrap();
        assert!(changed);
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains(BLOCK_START));
        assert!(content.contains(BLOCK_END));
        assert!(content.contains("verify_key"));
    }

    #[test]
    fn render_is_idempotent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("SECURITY.md");
        render(&path).unwrap();
        let changed = render(&path).unwrap();
        assert!(!changed);
    }

    #[test]
    fn render_preserves_user_content_outside_block() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("SECURITY.md");
        let user_content = "# My SECURITY policy\n\nUser-authored intro.\n\n";
        fs::write(&path, user_content).unwrap();
        render(&path).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("# My SECURITY policy"));
        assert!(content.contains("User-authored intro."));
        assert!(content.contains(BLOCK_START));
    }

    #[test]
    fn render_updates_existing_block_in_place() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("SECURITY.md");
        // Existing file with a stale Joy block surrounded by user content.
        let stale =
            format!("# Title\n\n{BLOCK_START}\nold content\n{BLOCK_END}\n\nFooter content.\n",);
        fs::write(&path, &stale).unwrap();
        render(&path).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("# Title"));
        assert!(content.contains("Footer content."));
        assert!(!content.contains("old content"));
        assert!(content.contains("verify_key"));
    }

    #[test]
    fn is_current_reports_false_for_missing_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("SECURITY.md");
        assert!(!is_current(&path).unwrap());
    }

    #[test]
    fn is_current_reports_true_after_render() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("SECURITY.md");
        render(&path).unwrap();
        assert!(is_current(&path).unwrap());
    }
}
