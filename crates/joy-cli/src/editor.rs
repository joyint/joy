// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Spawn the user's text editor on a tempfile and return what they
//! wrote. Used when a Joy command needs free-form input (`joy comment`
//! with no TEXT, future `joy edit --description` interactive form,
//! etc.). See JOY-00A5.
//!
//! Editor resolution order:
//! 1. `editor_flag` argument (passed via --editor on the command line)
//! 2. `editor` in the member's own joy config (local or global). The
//!    committed `.joy/config.defaults.yaml` is repository content and
//!    never names a command to run.
//! 3. `$VISUAL`
//! 4. `$EDITOR`
//!
//! Returns `Ok(Some(text))` on success, `Ok(None)` when the user
//! saved an empty file (interpreted as "abort", same convention as
//! git commit -e).

use anyhow::{anyhow, Result};
use std::fs;
use std::path::PathBuf;

pub fn edit_text(
    editor_flag: Option<&str>,
    initial: &str,
    file_suffix: &str,
) -> Result<Option<String>> {
    let editor = resolve_editor(editor_flag)?;

    let path = scratch_path(file_suffix);
    fs::write(&path, initial)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }

    let status = joy_process::command("sh")
        .arg("-c")
        .arg(format!("{editor} \"$@\""))
        .arg("sh")
        .arg(&path)
        .status()
        .map_err(|e| anyhow!("failed to launch editor '{editor}': {e}"))?;
    if !status.success() {
        let _ = fs::remove_file(&path);
        anyhow::bail!("editor '{editor}' exited with status {status}");
    }

    let content = fs::read_to_string(&path)?;
    let _ = fs::remove_file(&path);
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(trimmed.to_string()))
}

fn resolve_editor(editor_flag: Option<&str>) -> Result<String> {
    if let Some(e) = editor_flag {
        return Ok(e.to_string());
    }
    if let Some(e) = personal_editor(&joy_core::store::load_personal_config_value()) {
        return Ok(e);
    }
    if let Ok(e) = std::env::var("VISUAL") {
        if !e.trim().is_empty() {
            return Ok(e);
        }
    }
    if let Ok(e) = std::env::var("EDITOR") {
        if !e.trim().is_empty() {
            return Ok(e);
        }
    }
    Err(anyhow!(
        "no editor configured. Set one of: --editor <cmd>, \
         `joy config set editor <cmd>`, $VISUAL, or $EDITOR."
    ))
}

fn scratch_path(suffix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("joy-edit-{}-{suffix}", std::process::id()))
}

/// The editor the member set themselves, if any.
fn personal_editor(personal: &serde_json::Value) -> Option<String> {
    personal
        .get("editor")
        .and_then(|e| e.as_str())
        .filter(|e| !e.trim().is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_editor_comes_from_the_members_own_config_only() {
        let own = serde_json::json!({ "editor": "vim" });
        assert_eq!(personal_editor(&own), Some("vim".into()));
        assert_eq!(
            personal_editor(&serde_json::json!({ "editor": "  " })),
            None
        );
        assert_eq!(personal_editor(&serde_json::json!({})), None);
    }
}
