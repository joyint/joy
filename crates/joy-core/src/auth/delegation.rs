// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Stable per-(human, AI) delegation private keys (ADR-033).
//!
//! Each human keeps one Ed25519 private key per AI member they delegate to.
//! The private key never leaves the human's machine and is never read by the
//! AI: it is used only to sign delegation tokens at issuance time.
//!
//! The matching public key is stored in `project.yaml` under
//! `members[<human>].ai_delegations[<ai-member>].delegation_key`.
//!
//! On-disk layout:
//! `~/.local/state/joy/delegations/<project-acronym>/<sanitized-ai-member>.key`
//! The file contains the raw 32-byte Ed25519 seed, written with mode 0600.
//! It is an SSH-key-class secret: protect via filesystem permissions.

use std::path::PathBuf;

use super::session::dirs_state_dir;
use crate::error::JoyError;

const KEY_SIZE: usize = 32;

/// Directory holding delegation private keys for a project:
/// `~/.local/state/joy/delegations/<project>/`.
fn delegation_dir(project_id: &str) -> Result<PathBuf, JoyError> {
    Ok(dirs_state_dir()?
        .join("joy")
        .join("delegations")
        .join(sanitize(project_id)))
}

/// Path to the delegation private key file for a given (project, ai_member).
pub fn delegation_key_path(project_id: &str, ai_member: &str) -> Result<PathBuf, JoyError> {
    Ok(delegation_dir(project_id)?.join(format!("{}.key", sanitize(ai_member))))
}

/// Persist a 32-byte Ed25519 seed for the given (project, ai_member) with mode 0600.
pub fn save_delegation_key(
    project_id: &str,
    ai_member: &str,
    seed: &[u8; KEY_SIZE],
) -> Result<(), JoyError> {
    let dir = delegation_dir(project_id)?;
    std::fs::create_dir_all(&dir).map_err(|e| JoyError::CreateDir {
        path: dir.clone(),
        source: e,
    })?;
    let path = dir.join(format!("{}.key", sanitize(ai_member)));
    std::fs::write(&path, seed).map_err(|e| JoyError::WriteFile {
        path: path.clone(),
        source: e,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&path, perms).map_err(|e| JoyError::WriteFile {
            path: path.clone(),
            source: e,
        })?;
    }
    Ok(())
}

/// Load the 32-byte Ed25519 seed for the given (project, ai_member), if present.
pub fn load_delegation_key(
    project_id: &str,
    ai_member: &str,
) -> Result<Option<[u8; KEY_SIZE]>, JoyError> {
    let path = delegation_key_path(project_id, ai_member)?;
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).map_err(|e| JoyError::ReadFile {
        path: path.clone(),
        source: e,
    })?;
    if bytes.len() != KEY_SIZE {
        return Err(JoyError::AuthFailed(format!(
            "delegation key file {} is corrupt: expected {} bytes, got {}",
            path.display(),
            KEY_SIZE,
            bytes.len()
        )));
    }
    let mut seed = [0u8; KEY_SIZE];
    seed.copy_from_slice(&bytes);
    Ok(Some(seed))
}

/// Remove the delegation private key for a given (project, ai_member). No-op if absent.
pub fn remove_delegation_key(project_id: &str, ai_member: &str) -> Result<(), JoyError> {
    let path = delegation_key_path(project_id, ai_member)?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| JoyError::WriteFile {
            path: path.clone(),
            source: e,
        })?;
    }
    Ok(())
}

/// Replace any character outside [a-zA-Z0-9_-] with '_' so member ids like
/// "ai:claude@joy" map to filesystem-safe names on every supported platform.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::tempdir;

    /// Tests in this module mutate process-global `XDG_STATE_HOME`. Cargo runs
    /// tests in parallel by default, so without serialization concurrent tests
    /// would observe each other's tempdir overrides. The mutex confines the
    /// env-var manipulation to one test at a time.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_state_dir<F: FnOnce()>(f: F) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        // SAFETY: serialized via ENV_LOCK above
        unsafe { std::env::set_var("XDG_STATE_HOME", dir.path()) };
        f();
        // SAFETY: serialized via ENV_LOCK above
        unsafe { std::env::remove_var("XDG_STATE_HOME") };
    }

    #[test]
    fn sanitize_replaces_special_chars() {
        assert_eq!(sanitize("ai:claude@joy"), "ai_claude_joy");
        assert_eq!(sanitize("plain"), "plain");
        assert_eq!(sanitize("with-dash_und.dot"), "with-dash_und_dot");
    }

    #[test]
    fn save_load_roundtrip() {
        with_state_dir(|| {
            let seed = [42u8; 32];
            save_delegation_key("TST", "ai:claude@joy", &seed).unwrap();
            let loaded = load_delegation_key("TST", "ai:claude@joy")
                .unwrap()
                .unwrap();
            assert_eq!(loaded, seed);
        });
    }

    #[test]
    fn load_missing_returns_none() {
        with_state_dir(|| {
            let res = load_delegation_key("TST", "ai:absent@joy").unwrap();
            assert!(res.is_none());
        });
    }

    #[test]
    fn remove_deletes_file() {
        with_state_dir(|| {
            let seed = [1u8; 32];
            save_delegation_key("TST", "ai:claude@joy", &seed).unwrap();
            assert!(load_delegation_key("TST", "ai:claude@joy")
                .unwrap()
                .is_some());
            remove_delegation_key("TST", "ai:claude@joy").unwrap();
            assert!(load_delegation_key("TST", "ai:claude@joy")
                .unwrap()
                .is_none());
        });
    }

    #[test]
    fn remove_missing_is_noop() {
        with_state_dir(|| {
            remove_delegation_key("TST", "ai:never@joy").unwrap();
        });
    }

    #[test]
    fn projects_are_isolated() {
        with_state_dir(|| {
            let seed_a = [7u8; 32];
            let seed_b = [9u8; 32];
            save_delegation_key("AAA", "ai:claude@joy", &seed_a).unwrap();
            save_delegation_key("BBB", "ai:claude@joy", &seed_b).unwrap();
            assert_eq!(
                load_delegation_key("AAA", "ai:claude@joy").unwrap(),
                Some(seed_a)
            );
            assert_eq!(
                load_delegation_key("BBB", "ai:claude@joy").unwrap(),
                Some(seed_b)
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn file_mode_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        with_state_dir(|| {
            let seed = [3u8; 32];
            save_delegation_key("TST", "ai:claude@joy", &seed).unwrap();
            let path = delegation_key_path("TST", "ai:claude@joy").unwrap();
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        });
    }

    #[test]
    fn corrupt_file_rejected() {
        with_state_dir(|| {
            let dir = delegation_dir("TST").unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("ai_claude_joy.key");
            std::fs::write(&path, b"too-short").unwrap();
            let res = load_delegation_key("TST", "ai:claude@joy");
            assert!(res.is_err());
        });
    }
}
