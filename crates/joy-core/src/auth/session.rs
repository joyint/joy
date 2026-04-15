// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Session management for authenticated Joy operations.
//!
//! Sessions are time-limited tokens stored locally in `~/.config/joy/sessions/`.
//! They prove that the user has entered their passphrase and derived the correct
//! identity key within the configured time window.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use super::sign::{IdentityKeypair, PublicKey};
use crate::error::JoyError;

/// Claims encoded in a session token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionClaims {
    pub member: String,
    pub project_id: String,
    pub created: DateTime<Utc>,
    pub expires: DateTime<Utc>,
    /// For AI sessions: the token_key that was used to create this session.
    /// Used to invalidate sessions when the token is revoked/replaced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_key: Option<String>,
    /// Terminal device at session creation (e.g. "/dev/pts/1").
    /// Human sessions are only valid from the same terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tty: Option<String>,
}

/// A session token: claims + Ed25519 signature.
#[derive(Debug, Serialize, Deserialize)]
pub struct SessionToken {
    pub claims: SessionClaims,
    /// Hex-encoded Ed25519 signature over the serialized claims.
    pub signature: String,
}

/// Default session duration: 24 hours.
const DEFAULT_TTL_HOURS: i64 = 24;

/// Detect the current terminal device for session binding.
///
/// Returns a unique identifier for the terminal window/tab:
/// - Unix: TTY device path (e.g. "/dev/pts/1") via libc::ttyname
/// - Windows Terminal: WT_SESSION GUID (unique per tab/pane)
/// - No terminal (CI, cron, etc.): None
pub fn current_tty() -> Option<String> {
    // Windows Terminal sets WT_SESSION to a unique GUID per tab/pane
    if let Ok(wt) = std::env::var("WT_SESSION") {
        if !wt.is_empty() {
            return Some(format!("wt:{wt}"));
        }
    }

    #[cfg(unix)]
    {
        // SAFETY: ttyname returns a pointer to a static buffer.
        // We immediately copy it into a Rust String.
        let ptr = unsafe { libc::ttyname(0) };
        if !ptr.is_null() {
            let cstr = unsafe { std::ffi::CStr::from_ptr(ptr) };
            if let Ok(s) = cstr.to_str() {
                return Some(s.to_string());
            }
        }
    }

    None
}

/// Create a session token signed by the identity keypair.
pub fn create_session(
    keypair: &IdentityKeypair,
    member: &str,
    project_id: &str,
    ttl: Option<Duration>,
) -> SessionToken {
    create_session_with_token_key(keypair, member, project_id, ttl, None)
}

/// Create a session for an AI member, binding it to a specific token_key.
pub fn create_session_for_ai(
    keypair: &IdentityKeypair,
    member: &str,
    project_id: &str,
    ttl: Option<Duration>,
    token_key: &str,
) -> SessionToken {
    create_session_with_token_key(
        keypair,
        member,
        project_id,
        ttl,
        Some(token_key.to_string()),
    )
}

fn create_session_with_token_key(
    keypair: &IdentityKeypair,
    member: &str,
    project_id: &str,
    ttl: Option<Duration>,
    token_key: Option<String>,
) -> SessionToken {
    let now = Utc::now();
    let ttl = ttl.unwrap_or_else(|| Duration::hours(DEFAULT_TTL_HOURS));
    // Capture TTY for terminal-bound sessions (both human and AI).
    let tty = current_tty();
    let claims = SessionClaims {
        member: member.to_string(),
        project_id: project_id.to_string(),
        created: now,
        expires: now + ttl,
        token_key,
        tty,
    };
    let claims_json = serde_json::to_string(&claims).expect("claims serialize");
    let signature = keypair.sign(claims_json.as_bytes());
    SessionToken {
        claims,
        signature: hex::encode(signature),
    }
}

/// Validate a session token against a public key and project ID.
pub fn validate_session(
    token: &SessionToken,
    public_key: &PublicKey,
    project_id: &str,
) -> Result<SessionClaims, JoyError> {
    // Check project match
    if token.claims.project_id != project_id {
        return Err(JoyError::AuthFailed(
            "session belongs to a different project".into(),
        ));
    }

    // Check expiry
    if Utc::now() > token.claims.expires {
        return Err(JoyError::AuthFailed(
            "session expired, run `joy auth` to re-authenticate".into(),
        ));
    }

    // Verify signature
    let claims_json = serde_json::to_string(&token.claims).expect("claims serialize");
    let signature =
        hex::decode(&token.signature).map_err(|e| JoyError::AuthFailed(format!("{e}")))?;
    public_key.verify(claims_json.as_bytes(), &signature)?;

    Ok(token.claims.clone())
}

/// Directory for session files: `~/.local/state/joy/sessions/`
fn session_dir() -> Result<PathBuf, JoyError> {
    let state_dir = dirs_state_dir()?;
    Ok(state_dir.join("joy").join("sessions"))
}

/// Session filename: SHA-256 hash of project_id + member.
/// Deterministic but not human-readable (privacy).
fn session_filename(project_id: &str, member: &str) -> String {
    format!("{}.json", session_id(project_id, member))
}

/// The session ID: a short, deterministic, opaque identifier for a session.
/// Used as `JOY_SESSION` environment variable (SSH-agent pattern).
pub fn session_id(project_id: &str, member: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update(b":");
    hasher.update(member.as_bytes());
    let hash = hasher.finalize();
    hex::encode(&hash[..12])
}

/// Save a session token to disk.
pub fn save_session(project_id: &str, token: &SessionToken) -> Result<(), JoyError> {
    let dir = session_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| JoyError::CreateDir {
        path: dir.clone(),
        source: e,
    })?;
    let path = dir.join(session_filename(project_id, &token.claims.member));
    let json = serde_json::to_string_pretty(token).expect("session serialize");
    std::fs::write(&path, &json).map_err(|e| JoyError::WriteFile {
        path: path.clone(),
        source: e,
    })?;
    // Restrict to owner-only (session files contain signed claims)
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

/// Load a session token from disk for a specific member, if it exists.
pub fn load_session(project_id: &str, member: &str) -> Result<Option<SessionToken>, JoyError> {
    let dir = session_dir()?;
    let path = dir.join(session_filename(project_id, member));
    if !path.exists() {
        return Ok(None);
    }
    let json = std::fs::read_to_string(&path).map_err(|e| JoyError::ReadFile {
        path: path.clone(),
        source: e,
    })?;
    let token: SessionToken =
        serde_json::from_str(&json).map_err(|e| JoyError::AuthFailed(format!("{e}")))?;
    Ok(Some(token))
}

/// Load a session by its opaque ID (the JOY_SESSION value).
pub fn load_session_by_id(id: &str) -> Result<Option<SessionToken>, JoyError> {
    let dir = session_dir()?;
    let path = dir.join(format!("{id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let json = std::fs::read_to_string(&path).map_err(|e| JoyError::ReadFile {
        path: path.clone(),
        source: e,
    })?;
    let token: SessionToken =
        serde_json::from_str(&json).map_err(|e| JoyError::AuthFailed(format!("{e}")))?;
    Ok(Some(token))
}

/// Remove a session token from disk for a specific member.
pub fn remove_session(project_id: &str, member: &str) -> Result<(), JoyError> {
    let dir = session_dir()?;
    let path = dir.join(session_filename(project_id, member));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| JoyError::WriteFile { path, source: e })?;
    }
    Ok(())
}

/// Derive a stable project ID from project name and acronym.
pub fn project_id(root: &Path) -> Result<String, JoyError> {
    let project = crate::store::load_project(root)?;
    Ok(project
        .acronym
        .unwrap_or_else(|| project.name.to_lowercase().replace(' ', "-")))
}

pub(super) fn dirs_state_dir() -> Result<PathBuf, JoyError> {
    // Use XDG_STATE_HOME or ~/.local/state
    if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        return Ok(PathBuf::from(xdg));
    }
    if let Ok(home) = std::env::var("HOME") {
        return Ok(PathBuf::from(home).join(".local").join("state"));
    }
    Err(JoyError::AuthFailed(
        "cannot determine state directory".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{derive, sign};
    use tempfile::tempdir;

    const TEST_PASSPHRASE: &str = "correct horse battery staple extra words";

    fn test_keypair() -> (sign::IdentityKeypair, sign::PublicKey) {
        let salt = derive::Salt::from_hex(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        let key = derive::derive_key(TEST_PASSPHRASE, &salt).unwrap();
        let kp = sign::IdentityKeypair::from_derived_key(&key);
        let pk = kp.public_key();
        (kp, pk)
    }

    #[test]
    fn create_and_validate_session() {
        let (kp, pk) = test_keypair();
        let token = create_session(&kp, "test@example.com", "TST", None);
        let claims = validate_session(&token, &pk, "TST").unwrap();
        assert_eq!(claims.member, "test@example.com");
        assert_eq!(claims.project_id, "TST");
    }

    #[test]
    fn expired_session_rejected() {
        let (kp, pk) = test_keypair();
        let token = create_session(&kp, "test@example.com", "TST", Some(Duration::seconds(-1)));
        assert!(validate_session(&token, &pk, "TST").is_err());
    }

    #[test]
    fn wrong_project_rejected() {
        let (kp, pk) = test_keypair();
        let token = create_session(&kp, "test@example.com", "TST", None);
        assert!(validate_session(&token, &pk, "OTHER").is_err());
    }

    #[test]
    fn tampered_session_rejected() {
        let (kp, pk) = test_keypair();
        let mut token = create_session(&kp, "test@example.com", "TST", None);
        token.claims.member = "attacker@evil.com".into();
        assert!(validate_session(&token, &pk, "TST").is_err());
    }

    #[test]
    fn save_load_roundtrip() {
        let (kp, pk) = test_keypair();
        let token = create_session(&kp, "test@example.com", "TST", None);

        let dir = tempdir().unwrap();
        // Override session dir via env
        // SAFETY: test is single-threaded, setting env var for session dir override
        unsafe { std::env::set_var("XDG_STATE_HOME", dir.path()) };

        save_session("TST", &token).unwrap();
        let loaded = load_session("TST", "test@example.com").unwrap().unwrap();
        let claims = validate_session(&loaded, &pk, "TST").unwrap();
        assert_eq!(claims.member, "test@example.com");

        remove_session("TST", "test@example.com").unwrap();
        assert!(load_session("TST", "test@example.com").unwrap().is_none());

        // SAFETY: test cleanup
        unsafe { std::env::remove_var("XDG_STATE_HOME") };
    }
}
