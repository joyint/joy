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

use super::{IdentityKeypair, PublicKey};
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
    /// Anonymous mode (ADR-042): the hex-encoded members.yaml zone key, cached
    /// for the life of the session so any command can resolve opaque ids to
    /// e-mails without re-entering the passphrase (the concept's "session ⇒
    /// resolvable"). Auxiliary local state, not part of the signed claims; the
    /// session file is owner-only (0600), the same trust boundary as the
    /// session credential itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub members_zone_key: Option<String>,
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
///
/// Per ADR-041 §6, the `token_expires` is an upper bound on the session's
/// own expiry: `session.expires = min(session_ttl, token_expires)`. When
/// the AI redeems a 30-minute Crypt token, the session must die after 30
/// minutes too, regardless of the configured session TTL.
pub fn create_session_for_ai(
    ephemeral_keypair: &IdentityKeypair,
    member: &str,
    project_id: &str,
    ttl: Option<Duration>,
    delegation_key: &str,
    token_expires: Option<DateTime<Utc>>,
) -> SessionToken {
    let now = Utc::now();
    let ttl = ttl.unwrap_or_else(|| Duration::hours(DEFAULT_TTL_HOURS));
    let session_expiry = now + ttl;
    let expires = match token_expires {
        Some(token_exp) if token_exp < session_expiry => token_exp,
        _ => session_expiry,
    };
    let claims = SessionClaims {
        member: member.to_string(),
        project_id: project_id.to_string(),
        created: now,
        expires,
        token_key: Some(delegation_key.to_string()),
        session_public_key: Some(ephemeral_keypair.public_key().to_hex()),
        tty: None,
    };
    let claims_json = serde_json::to_string(&claims).expect("claims serialize");
    let signature = ephemeral_keypair.sign(claims_json.as_bytes());
    SessionToken {
        claims,
        signature: hex::encode(signature),
        members_zone_key: None,
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
        members_zone_key: None,
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
const DELEGATION_PRIVATE_LEN: usize = 32;

/// Encode a `JOY_SESSION` env var value. Backward-compatible with sessions
/// that carry only the ephemeral session key; pass `delegation_private =
/// None` for that case.
///
/// Layout (base64-encoded):
/// - Auth-only:    `[sid 12][session_priv 32]` (44 bytes)
/// - Auth + Crypt: `[sid 12][session_priv 32][delegation_priv 32]` (76 bytes)
///
/// Per ADR-041 §5, the delegation private key is included exactly when the
/// originating token had `crypt` scope. The AI's joy commands read the
/// delegation key from this env var to unwrap zone keys; it never lives on
/// disk.
pub fn encode_session_env(sid_hex: &str, ephemeral_private: &[u8; SESSION_PRIVATE_LEN]) -> String {
    encode_session_env_full(sid_hex, ephemeral_private, None)
}

/// Encode a `JOY_SESSION` env var value with an optional embedded
/// delegation private key (ADR-041 §5).
pub fn encode_session_env_full(
    sid_hex: &str,
    ephemeral_private: &[u8; SESSION_PRIVATE_LEN],
    delegation_private: Option<&[u8; DELEGATION_PRIVATE_LEN]>,
) -> String {
    let sid_bytes = hex::decode(sid_hex).expect("session id must be valid hex");
    assert_eq!(
        sid_bytes.len(),
        SESSION_ID_LEN,
        "session id length mismatch"
    );
    let total_len = SESSION_ID_LEN
        + SESSION_PRIVATE_LEN
        + if delegation_private.is_some() {
            DELEGATION_PRIVATE_LEN
        } else {
            0
        };
    let mut payload = Vec::with_capacity(total_len);
    payload.extend_from_slice(&sid_bytes);
    payload.extend_from_slice(ephemeral_private);
    if let Some(dpk) = delegation_private {
        payload.extend_from_slice(dpk);
    }
    use base64ct::{Base64, Encoding};
    format!("{SESSION_ENV_PREFIX}{}", Base64::encode_string(&payload))
}

/// Parse a `JOY_SESSION` env var value produced by `encode_session_env`.
/// Returns `(sid_hex, ephemeral_private_bytes)` or None on malformed input.
/// Sessions with an embedded delegation private key (Crypt scope) parse
/// successfully here too; use `parse_session_env_full` to access the
/// delegation key.
pub fn parse_session_env(env_value: &str) -> Option<(String, [u8; SESSION_PRIVATE_LEN])> {
    let (sid, session_priv, _) = parse_session_env_full(env_value)?;
    Some((sid, session_priv))
}

/// Parse a `JOY_SESSION` env var value, returning the session id, the
/// ephemeral session private key, and (if the originating token was
/// Crypt-scoped) the delegation private key embedded for zone-key unwrap
/// (ADR-041 §5).
pub fn parse_session_env_full(
    env_value: &str,
) -> Option<(
    String,
    [u8; SESSION_PRIVATE_LEN],
    Option<[u8; DELEGATION_PRIVATE_LEN]>,
)> {
    let encoded = env_value.strip_prefix(SESSION_ENV_PREFIX)?;
    use base64ct::{Base64, Encoding};
    let payload = Base64::decode_vec(encoded).ok()?;
    let auth_only_len = SESSION_ID_LEN + SESSION_PRIVATE_LEN;
    let with_crypt_len = auth_only_len + DELEGATION_PRIVATE_LEN;
    if payload.len() != auth_only_len && payload.len() != with_crypt_len {
        return None;
    }
    let sid_hex = hex::encode(&payload[..SESSION_ID_LEN]);
    let mut session_priv = [0u8; SESSION_PRIVATE_LEN];
    session_priv.copy_from_slice(&payload[SESSION_ID_LEN..auth_only_len]);
    let delegation_priv = if payload.len() == with_crypt_len {
        let mut dpk = [0u8; DELEGATION_PRIVATE_LEN];
        dpk.copy_from_slice(&payload[auth_only_len..]);
        Some(dpk)
    } else {
        None
    };
    Some((sid_hex, session_priv, delegation_priv))
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

/// Whether a non-expired session for `member` exists on disk for this project.
///
/// Used by `joy ai reset` to warn before removing a member that is still in
/// active use, instead of silently invalidating a live session.
pub fn has_active_session(project_id: &str, member: &str) -> bool {
    matches!(
        load_session(project_id, member),
        Ok(Some(token)) if token.claims.expires > Utc::now()
    )
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
    Ok(project_id_of(&project))
}

/// Same as `project_id` but operates on an in-memory `Project`. Useful when
/// the caller needs both the pre- and post-mutation id (e.g. acronym
/// rename migrations) without re-reading the yaml.
pub fn project_id_of(project: &crate::model::Project) -> String {
    project
        .acronym
        .clone()
        .unwrap_or_else(|| project.name.to_lowercase().replace(' ', "-"))
}

pub(super) fn dirs_state_dir() -> Result<PathBuf, JoyError> {
    // State dir: $XDG_STATE_HOME, else on Windows %LOCALAPPDATA%
    // (fallback %USERPROFILE%\AppData\Local), else on Unix $HOME/.local/state.
    // Shared resolver lives in `store` so config and state paths stay in sync.
    crate::store::resolve_base_dir(
        std::env::var("XDG_STATE_HOME").ok(),
        std::env::var("LOCALAPPDATA").ok(),
        std::env::var("HOME").ok(),
        std::env::var("USERPROFILE").ok(),
        cfg!(windows),
        "Local",
        ".local/state",
    )
    .ok_or_else(|| JoyError::AuthFailed("cannot determine state directory".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{derive_key, IdentityKeypair, PublicKey, Salt};
    use tempfile::tempdir;

    const TEST_PASSPHRASE: &str = "correct horse battery staple extra words";

    fn test_keypair() -> (IdentityKeypair, PublicKey) {
        let salt =
            Salt::from_hex("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .unwrap();
        let key = derive_key(TEST_PASSPHRASE, &salt).unwrap();
        let kp = IdentityKeypair::from_derived_key(&key);
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
        let ephemeral = IdentityKeypair::from_random();
        let ephemeral_pk = ephemeral.public_key().to_hex();
        let token = create_session_for_ai(&ephemeral, "ai:claude@joy", "TST", None, "dkey", None);
        assert_eq!(
            token.claims.session_public_key.as_deref(),
            Some(ephemeral_pk.as_str())
        );
        assert_eq!(token.claims.token_key.as_deref(), Some("dkey"));
        // Ensure the session signature validates against the ephemeral public key.
        let pk = PublicKey::from_hex(&ephemeral_pk).unwrap();
        validate_session(&token, &pk, "TST").unwrap();
    }

    #[test]
    fn ai_session_clamped_to_token_expiry() {
        // ADR-041 §6: a 30-minute token must not produce a 24h session.
        let ephemeral = IdentityKeypair::from_random();
        let token_expires = Utc::now() + Duration::minutes(30);
        let token = create_session_for_ai(
            &ephemeral,
            "ai:claude@joy",
            "TST",
            None,
            "dkey",
            Some(token_expires),
        );
        // Session expiry should equal token_expires (within a tiny window).
        let delta = (token.claims.expires - token_expires).num_seconds().abs();
        assert!(delta < 2, "session expiry should match token expiry");
    }

    #[test]
    fn ai_session_uses_session_ttl_when_token_lives_longer() {
        let ephemeral = IdentityKeypair::from_random();
        let token_expires = Utc::now() + Duration::days(7);
        let token = create_session_for_ai(
            &ephemeral,
            "ai:claude@joy",
            "TST",
            Some(Duration::hours(1)),
            "dkey",
            Some(token_expires),
        );
        // Session expiry should be ~1h, not 7 days.
        let session_ttl = token.claims.expires - token.claims.created;
        assert!(
            session_ttl <= Duration::hours(1),
            "session must respect its own TTL when token lives longer"
        );
    }

    #[test]
    fn session_env_full_roundtrip_with_delegation() {
        let sid = "0123456789abcdef01234567";
        let session_priv = [7u8; 32];
        let delegation_priv = [9u8; 32];
        let encoded = encode_session_env_full(sid, &session_priv, Some(&delegation_priv));
        let (decoded_sid, decoded_session, decoded_delegation) =
            parse_session_env_full(&encoded).unwrap();
        assert_eq!(decoded_sid, sid);
        assert_eq!(decoded_session, session_priv);
        assert_eq!(decoded_delegation, Some(delegation_priv));
    }

    #[test]
    fn session_env_legacy_auth_only_still_parses() {
        let sid = "0123456789abcdef01234567";
        let session_priv = [7u8; 32];
        let encoded = encode_session_env(sid, &session_priv);
        let (decoded_sid, decoded_session, decoded_delegation) =
            parse_session_env_full(&encoded).unwrap();
        assert_eq!(decoded_sid, sid);
        assert_eq!(decoded_session, session_priv);
        assert!(decoded_delegation.is_none());
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
