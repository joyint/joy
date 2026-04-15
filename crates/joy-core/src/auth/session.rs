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
    /// For AI sessions: the delegation_key this session was bound to at creation.
    /// Rotating the delegation invalidates the session. Field name kept as
    /// `token_key` for on-disk compatibility with already-written sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_key: Option<String>,
    /// For AI sessions (ADR-033): the ephemeral public key whose matching
    /// private key lives only in the `JOY_SESSION` env var. Validation
    /// requires the caller to possess that private key, binding the session
    /// to the terminal environment it was created in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_public_key: Option<String>,
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

/// Create a session for an AI member with an ephemeral keypair (ADR-033).
///
/// The `ephemeral_keypair`'s public counterpart is recorded in the session
/// claims; the matching private key must live in the `JOY_SESSION` env var
/// of the caller. `delegation_key` is the hex-encoded public key of the
/// stable ai_delegations entry; rotating that key invalidates the session.
pub fn create_session_for_ai(
    ephemeral_keypair: &IdentityKeypair,
    member: &str,
    project_id: &str,
    ttl: Option<Duration>,
    delegation_key: &str,
) -> SessionToken {
    let now = Utc::now();
    let ttl = ttl.unwrap_or_else(|| Duration::hours(DEFAULT_TTL_HOURS));
    let claims = SessionClaims {
        member: member.to_string(),
        project_id: project_id.to_string(),
        created: now,
        expires: now + ttl,
        token_key: Some(delegation_key.to_string()),
        session_public_key: Some(ephemeral_keypair.public_key().to_hex()),
        tty: None,
    };
    let claims_json = serde_json::to_string(&claims).expect("claims serialize");
    let signature = ephemeral_keypair.sign(claims_json.as_bytes());
    SessionToken {
        claims,
        signature: hex::encode(signature),
    }
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
    // Human sessions remain TTY-bound (ADR-023); AI sessions use the
    // ephemeral-keypair path above.
    let tty = current_tty();
    let claims = SessionClaims {
        member: member.to_string(),
        project_id: project_id.to_string(),
        created: now,
        expires: now + ttl,
        token_key,
        session_public_key: None,
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
/// Used as the filename stub for the session file and as part of the
/// `JOY_SESSION` env var payload.
pub fn session_id(project_id: &str, member: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update(b":");
    hasher.update(member.as_bytes());
    let hash = hasher.finalize();
    hex::encode(&hash[..SESSION_ID_LEN])
}

/// Prefix for the `JOY_SESSION` env var value (ADR-033).
pub const SESSION_ENV_PREFIX: &str = "joy_s_";
const SESSION_ID_LEN: usize = 12;
const SESSION_PRIVATE_LEN: usize = 32;

/// Encode a `JOY_SESSION` env var value from the session id (hex) and the
/// 32-byte ephemeral private key. Layout of the decoded payload:
/// `[sid_raw 12 bytes][ephemeral_private 32 bytes]`, base64-encoded.
pub fn encode_session_env(sid_hex: &str, ephemeral_private: &[u8; SESSION_PRIVATE_LEN]) -> String {
    let sid_bytes = hex::decode(sid_hex).expect("session id must be valid hex");
    assert_eq!(
        sid_bytes.len(),
        SESSION_ID_LEN,
        "session id length mismatch"
    );
    let mut payload = Vec::with_capacity(SESSION_ID_LEN + SESSION_PRIVATE_LEN);
    payload.extend_from_slice(&sid_bytes);
    payload.extend_from_slice(ephemeral_private);
    use base64ct::{Base64, Encoding};
    format!("{SESSION_ENV_PREFIX}{}", Base64::encode_string(&payload))
}

/// Parse a `JOY_SESSION` env var value produced by `encode_session_env`.
/// Returns `(sid_hex, ephemeral_private_bytes)` or None on malformed input.
pub fn parse_session_env(env_value: &str) -> Option<(String, [u8; SESSION_PRIVATE_LEN])> {
    let encoded = env_value.strip_prefix(SESSION_ENV_PREFIX)?;
    use base64ct::{Base64, Encoding};
    let payload = Base64::decode_vec(encoded).ok()?;
    if payload.len() != SESSION_ID_LEN + SESSION_PRIVATE_LEN {
        return None;
    }
    let sid_hex = hex::encode(&payload[..SESSION_ID_LEN]);
    let mut private = [0u8; SESSION_PRIVATE_LEN];
    private.copy_from_slice(&payload[SESSION_ID_LEN..]);
    Some((sid_hex, private))
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
    fn session_env_roundtrip() {
        let sid = "0123456789abcdef01234567";
        let private = [7u8; 32];
        let encoded = encode_session_env(sid, &private);
        assert!(encoded.starts_with(SESSION_ENV_PREFIX));
        let (decoded_sid, decoded_priv) = parse_session_env(&encoded).unwrap();
        assert_eq!(decoded_sid, sid);
        assert_eq!(decoded_priv, private);
    }

    #[test]
    fn parse_session_env_rejects_bad_inputs() {
        assert!(parse_session_env("no_prefix_value").is_none());
        assert!(parse_session_env("joy_s_!!!").is_none());
        // wrong length
        use base64ct::{Base64, Encoding};
        let short = format!("{SESSION_ENV_PREFIX}{}", Base64::encode_string(&[1u8; 10]));
        assert!(parse_session_env(&short).is_none());
    }

    #[test]
    fn ai_session_carries_ephemeral_public_key() {
        let ephemeral = sign::IdentityKeypair::from_random();
        let ephemeral_pk = ephemeral.public_key().to_hex();
        let token = create_session_for_ai(&ephemeral, "ai:claude@joy", "TST", None, "dkey");
        assert_eq!(
            token.claims.session_public_key.as_deref(),
            Some(ephemeral_pk.as_str())
        );
        assert_eq!(token.claims.token_key.as_deref(), Some("dkey"));
        // Ensure the session signature validates against the ephemeral public key.
        let pk = sign::PublicKey::from_hex(&ephemeral_pk).unwrap();
        validate_session(&token, &pk, "TST").unwrap();
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
