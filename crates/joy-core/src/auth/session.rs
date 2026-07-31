// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Session management for authenticated Joy operations.
//!
//! Sessions are time-limited tokens stored locally in
//! `~/.local/state/joy/sessions/` (honoring `XDG_STATE_HOME`). They prove
//! that the user has entered their passphrase and derived the correct
//! identity key within the configured time window.
//!
//! Human sessions occupy one deterministic slot per (project, member):
//! re-authenticating replaces the previous session. AI sessions get one
//! file per session, keyed by the ephemeral session public key, because
//! every token redemption is an independent session (JOY-01E1-E7) and a
//! redemption in one terminal must not displace the session another
//! terminal is still using. Expired files are swept lazily on save.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use super::{IdentityKeypair, PublicKey};
use crate::error::JoyError;
use crate::model::project::is_ai_member;

/// Claims encoded in a session token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionClaims {
    pub member: String,
    pub project_id: String,
    pub created: DateTime<Utc>,
    pub expires: DateTime<Utc>,
    /// For AI sessions: the delegation_verifier this session was bound to
    /// at creation. Rotating the delegation invalidates the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation_key: Option<String>,
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
    // ------------------------------------------------------------------
    // SIGNATURE COMPATIBILITY: signatures verify over the serde_json
    // serialization of these claims, so field declaration order IS the
    // signed byte stream. New claims must be `Option` +
    // `skip_serializing_if = "Option::is_none"` and APPENDED here, never
    // inserted or reordered: sessions written before a field existed then
    // still serialize (and verify) byte-identically. Renaming or removing
    // a field is allowed exactly BECAUSE it breaks old files: sessions
    // are ephemeral, and a shape change simply expires them into a fresh
    // `joy auth` instead of keeping a read path alive.
    // ------------------------------------------------------------------
    /// The human whose delegation this session acts under, recorded at
    /// redemption (F2, JI-0175-B0), so every write names the person
    /// behind the AI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegated_by: Option<String>,
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
    create_session_with_delegation_key(keypair, member, project_id, ttl, None)
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
    delegated_by: Option<String>,
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
        delegation_key: Some(delegation_key.to_string()),
        session_public_key: Some(ephemeral_keypair.public_key().to_hex()),
        tty: None,
        // The delegating human recorded at redemption (F2, JI-0175-B0): a
        // token-redeemed session used to leave this None, and the identity
        // resolver then guessed the delegator from the local git e-mail —
        // which inside an agent container is the image's fake identity. It
        // now travels in the signed claims so `delegated-by:` is correct
        // wherever the session runs.
        delegated_by,
    };
    let claims_json = serde_json::to_string(&claims).expect("claims serialize");
    let signature = ephemeral_keypair.sign(claims_json.as_bytes());
    SessionToken {
        claims,
        signature: hex::encode(signature),
        members_zone_key: None,
    }
}

fn create_session_with_delegation_key(
    keypair: &IdentityKeypair,
    member: &str,
    project_id: &str,
    ttl: Option<Duration>,
    delegation_key: Option<String>,
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
        delegation_key,
        session_public_key: None,
        tty,
        delegated_by: None,
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

/// The session ID of the single per-member slot: deterministic, opaque.
/// Human sessions live under this id. AI sessions use
/// [`session_storage_id`] instead, which mixes in the ephemeral session
/// public key so each session gets its own file.
pub fn session_id(project_id: &str, member: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update(b":");
    hasher.update(member.as_bytes());
    let hash = hasher.finalize();
    hex::encode(&hash[..SESSION_ID_LEN])
}

/// The storage ID of a session: the filename stub of its file on disk and
/// the sid carried in the `JOY_SESSION` env payload.
///
/// AI sessions (token-redeemed and job-bound alike) are keyed by the
/// ephemeral session public key, so every session occupies its own file
/// and concurrent sessions for the same member coexist (JOY-01E1-E7).
/// Human sessions keep the deterministic per-member slot: their lookup
/// runs by (project, member), not by an env-carried sid.
pub fn session_storage_id(project_id: &str, claims: &SessionClaims) -> String {
    match &claims.session_public_key {
        // AI sessions get a PER-SESSION file (keyed by the ephemeral
        // session public key), so an AI can hold several at once (one per
        // chat, and one per job round) and each survives independently
        // (JOY-01E1-E7).
        Some(session_pk) if is_ai_member(&claims.member) => {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(project_id.as_bytes());
            hasher.update(b":");
            hasher.update(claims.member.as_bytes());
            hasher.update(b":");
            hasher.update(session_pk.as_bytes());
            let hash = hasher.finalize();
            hex::encode(&hash[..SESSION_ID_LEN])
        }
        _ => session_id(project_id, &claims.member),
    }
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

/// Save a session token to disk, under its [`session_storage_id`].
///
/// Saving never displaces another live session: AI sessions land in their
/// own per-session file, human sessions replace only the caller's own
/// per-member slot. Expired session files are swept as a side effect so
/// the directory does not accumulate dead files.
pub fn save_session(project_id: &str, token: &SessionToken) -> Result<(), JoyError> {
    let dir = session_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| JoyError::CreateDir {
        path: dir.clone(),
        source: e,
    })?;
    sweep_expired_sessions(&dir);
    let path = dir.join(format!(
        "{}.json",
        session_storage_id(project_id, &token.claims)
    ));
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

/// Load the per-member slot session from disk, if it exists.
///
/// This resolves only the deterministic (project, member) slot — the home
/// of human sessions. AI sessions live in per-session files; resolve them
/// via [`load_session_by_id`] (env sid) or [`list_member_sessions`].
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

/// All session files belonging to (project, member), with their on-disk
/// paths: the per-member slot plus every per-session AI file. Unreadable
/// or foreign files are skipped; a missing directory yields an empty list.
pub fn list_member_sessions(
    project_id: &str,
    member: &str,
) -> Result<Vec<(PathBuf, SessionToken)>, JoyError> {
    let dir = session_dir()?;
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };
    let mut sessions = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(json) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(token) = serde_json::from_str::<SessionToken>(&json) else {
            continue;
        };
        if token.claims.project_id == project_id && token.claims.member == member {
            sessions.push((path, token));
        }
    }
    // Newest first, so "the" session of a member is the most recent one.
    sessions.sort_by_key(|(_, token)| std::cmp::Reverse(token.claims.created));
    Ok(sessions)
}

/// The session the current environment points at: parses `JOY_SESSION`,
/// loads the file it references, and returns it if it belongs to
/// (project, member). Possession of the matching ephemeral private key is
/// NOT checked here — that stays with `resolve_identity`; this is a
/// display/lookup helper.
pub fn current_env_session(project_id: &str, member: &str) -> Option<SessionToken> {
    let env_value = std::env::var("JOY_SESSION")
        .ok()
        .filter(|s| !s.is_empty())?;
    let (sid, _) = parse_session_env(&env_value)?;
    let token = load_session_by_id(&sid).ok().flatten()?;
    (token.claims.project_id == project_id && token.claims.member == member).then_some(token)
}

/// Remove every expired session file in `dir`. Best effort: unreadable or
/// unparseable files are left alone (they may belong to a newer joy).
fn sweep_expired_sessions(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = Utc::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(json) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(token) = serde_json::from_str::<SessionToken>(&json) else {
            continue;
        };
        if token.claims.expires < now {
            let _ = std::fs::remove_file(&path);
        }
    }
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
    let now = Utc::now();
    list_member_sessions(project_id, member)
        .map(|sessions| sessions.iter().any(|(_, t)| t.claims.expires > now))
        .unwrap_or(false)
}

/// Remove ALL of a member's sessions from disk for this project: the
/// per-member slot and every per-session AI file. Deauth and delegation
/// rotation mean "this member is signed out", not "one shell is".
pub fn remove_session(project_id: &str, member: &str) -> Result<(), JoyError> {
    let dir = session_dir()?;
    let slot = dir.join(session_filename(project_id, member));
    if slot.exists() {
        std::fs::remove_file(&slot).map_err(|e| JoyError::WriteFile {
            path: slot,
            source: e,
        })?;
    }
    for (path, _) in list_member_sessions(project_id, member)? {
        std::fs::remove_file(&path).map_err(|e| JoyError::WriteFile { path, source: e })?;
    }
    Ok(())
}

/// Remove a single session file, addressed by its storage id. Used to
/// drop one stale session (e.g. bound to a rotated delegation key)
/// without signing out the member's other sessions.
pub fn remove_session_by_id(id: &str) -> Result<(), JoyError> {
    let dir = session_dir()?;
    let path = dir.join(format!("{id}.json"));
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

/// The OS state base directory, publicly: hosts park per-tool agent
/// state (JI-017A-85 state_env) next to joy's own sessions instead of
/// re-deriving the platform rules.
pub fn state_base_dir() -> Result<PathBuf, JoyError> {
    dirs_state_dir()
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

    /// Serializes tests that mutate process-global env vars
    /// (`XDG_STATE_HOME`, `JOY_SESSION`): cargo runs tests on threads.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

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
        let token =
            create_session_for_ai(&ephemeral, "ai:claude@joy", "TST", None, "dkey", None, None);
        assert_eq!(
            token.claims.session_public_key.as_deref(),
            Some(ephemeral_pk.as_str())
        );
        assert_eq!(token.claims.delegation_key.as_deref(), Some("dkey"));
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
            None,
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
            None,
        );
        // Session expiry should be ~1h, not 7 days.
        let session_ttl = token.claims.expires - token.claims.created;
        assert!(
            session_ttl <= Duration::hours(1),
            "session must respect its own TTL when token lives longer"
        );
    }

    #[test]
    fn optional_claims_stay_absent_from_plain_sessions() {
        // The signature is over the serde_json byte stream: absent
        // optional claims must not serialize, or a session written
        // without them would re-serialize differently and break its own
        // signature.
        let (kp, pk) = test_keypair();
        let token = create_session(&kp, "test@example.com", "TST", None);
        let json = serde_json::to_string(&token.claims).unwrap();
        assert!(!json.contains("delegation_key"), "got: {json}");
        assert!(!json.contains("delegated_by"), "got: {json}");
        validate_session(&token, &pk, "TST").unwrap();
    }

    #[test]
    fn a_session_written_by_a_retired_shape_fails_verification() {
        // A file carrying retired claims (the platform-key era job_id /
        // issuer, or the pre-rename token_key) still parses — unknown
        // fields are ignored — but its signature no longer verifies over
        // the re-serialized claims, so it expires into a fresh joy auth
        // instead of being quietly honored.
        let json = r#"{
            "claims": {
                "member": "ai:claude@joy",
                "project_id": "TST",
                "created": "2026-01-01T00:00:00Z",
                "expires": "2099-01-02T00:00:00Z",
                "session_public_key": "aa",
                "token_key": "bb",
                "job_id": "JOB-1",
                "issuer": "platform"
            },
            "signature": "00"
        }"#;
        let token: SessionToken = serde_json::from_str(json).unwrap();
        assert!(token.claims.delegation_key.is_none());
        let (_kp, pk) = test_keypair();
        assert!(validate_session(&token, &pk, "TST").is_err());
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
        let _guard = env_lock();
        let (kp, pk) = test_keypair();
        let token = create_session(&kp, "test@example.com", "TST", None);

        let dir = tempdir().unwrap();
        // Override session dir via env
        // SAFETY: env mutation serialized via ENV_LOCK
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

    #[test]
    fn ai_sessions_coexist_one_file_per_session() {
        let _guard = env_lock();
        let dir = tempdir().unwrap();
        // SAFETY: env mutation serialized via ENV_LOCK
        unsafe { std::env::set_var("XDG_STATE_HOME", dir.path()) };

        let first_kp = IdentityKeypair::from_random();
        let second_kp = IdentityKeypair::from_random();
        let first =
            create_session_for_ai(&first_kp, "ai:claude@joy", "TST", None, "dkey", None, None);
        let second =
            create_session_for_ai(&second_kp, "ai:claude@joy", "TST", None, "dkey", None, None);
        save_session("TST", &first).unwrap();
        save_session("TST", &second).unwrap();

        let first_sid = session_storage_id("TST", &first.claims);
        let second_sid = session_storage_id("TST", &second.claims);
        assert_ne!(first_sid, second_sid, "each session gets its own id");

        // Saving the second session must not displace the first
        // (JOY-01E1-E7: redemptions are independent sessions).
        let loaded_first = load_session_by_id(&first_sid).unwrap().unwrap();
        assert_eq!(
            loaded_first.claims.session_public_key,
            first.claims.session_public_key
        );
        let loaded_second = load_session_by_id(&second_sid).unwrap().unwrap();
        assert_eq!(
            loaded_second.claims.session_public_key,
            second.claims.session_public_key
        );

        assert_eq!(
            list_member_sessions("TST", "ai:claude@joy").unwrap().len(),
            2
        );
        assert!(has_active_session("TST", "ai:claude@joy"));

        // remove_session signs the member out everywhere.
        remove_session("TST", "ai:claude@joy").unwrap();
        assert!(list_member_sessions("TST", "ai:claude@joy")
            .unwrap()
            .is_empty());
        assert!(!has_active_session("TST", "ai:claude@joy"));

        // SAFETY: test cleanup
        unsafe { std::env::remove_var("XDG_STATE_HOME") };
    }

    #[test]
    fn expired_sessions_swept_on_save() {
        let _guard = env_lock();
        let dir = tempdir().unwrap();
        // SAFETY: env mutation serialized via ENV_LOCK
        unsafe { std::env::set_var("XDG_STATE_HOME", dir.path()) };

        let expired_kp = IdentityKeypair::from_random();
        let expired = create_session_for_ai(
            &expired_kp,
            "ai:claude@joy",
            "TST",
            Some(Duration::seconds(-1)),
            "dkey",
            None,
            None,
        );
        save_session("TST", &expired).unwrap();

        let fresh_kp = IdentityKeypair::from_random();
        let fresh =
            create_session_for_ai(&fresh_kp, "ai:claude@joy", "TST", None, "dkey", None, None);
        save_session("TST", &fresh).unwrap();

        let sessions = list_member_sessions("TST", "ai:claude@joy").unwrap();
        assert_eq!(sessions.len(), 1, "expired session swept on save");
        assert_eq!(
            sessions[0].1.claims.session_public_key,
            fresh.claims.session_public_key
        );

        // SAFETY: test cleanup
        unsafe { std::env::remove_var("XDG_STATE_HOME") };
    }

    #[test]
    fn human_sessions_keep_single_slot() {
        let _guard = env_lock();
        let dir = tempdir().unwrap();
        // SAFETY: env mutation serialized via ENV_LOCK
        unsafe { std::env::set_var("XDG_STATE_HOME", dir.path()) };

        let (kp, _) = test_keypair();
        let first = create_session(&kp, "test@example.com", "TST", None);
        let second = create_session(&kp, "test@example.com", "TST", None);
        save_session("TST", &first).unwrap();
        save_session("TST", &second).unwrap();

        // Re-authenticating replaces the slot instead of accumulating.
        assert_eq!(
            list_member_sessions("TST", "test@example.com")
                .unwrap()
                .len(),
            1
        );
        assert!(load_session("TST", "test@example.com").unwrap().is_some());

        // SAFETY: test cleanup
        unsafe { std::env::remove_var("XDG_STATE_HOME") };
    }

    #[test]
    fn legacy_ai_session_slot_still_resolves_and_removes() {
        // Sessions written by a pre-JOY-01E1-E7-fix joy live under the
        // deterministic per-member filename, and live JOY_SESSION values
        // carry that legacy sid. They must stay resolvable across the
        // upgrade and disappear on remove_session.
        let _guard = env_lock();
        let tmp = tempdir().unwrap();
        // SAFETY: env mutation serialized via ENV_LOCK
        unsafe { std::env::set_var("XDG_STATE_HOME", tmp.path()) };

        let kp = IdentityKeypair::from_random();
        let token = create_session_for_ai(&kp, "ai:claude@joy", "TST", None, "dkey", None, None);
        let dir = session_dir().unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let legacy_sid = session_id("TST", "ai:claude@joy");
        std::fs::write(
            dir.join(format!("{legacy_sid}.json")),
            serde_json::to_string(&token).unwrap(),
        )
        .unwrap();

        assert!(load_session_by_id(&legacy_sid).unwrap().is_some());
        assert_eq!(
            list_member_sessions("TST", "ai:claude@joy").unwrap().len(),
            1
        );
        assert!(has_active_session("TST", "ai:claude@joy"));

        remove_session("TST", "ai:claude@joy").unwrap();
        assert!(load_session_by_id(&legacy_sid).unwrap().is_none());
        assert!(!has_active_session("TST", "ai:claude@joy"));

        // SAFETY: test cleanup
        unsafe { std::env::remove_var("XDG_STATE_HOME") };
    }

    #[test]
    fn current_env_session_resolves_the_env_referenced_session() {
        let _guard = env_lock();
        let dir = tempdir().unwrap();
        // SAFETY: env mutation serialized via ENV_LOCK
        unsafe { std::env::set_var("XDG_STATE_HOME", dir.path()) };

        let first_kp = IdentityKeypair::from_random();
        let second_kp = IdentityKeypair::from_random();
        let first =
            create_session_for_ai(&first_kp, "ai:claude@joy", "TST", None, "dkey", None, None);
        let second =
            create_session_for_ai(&second_kp, "ai:claude@joy", "TST", None, "dkey", None, None);
        save_session("TST", &first).unwrap();
        save_session("TST", &second).unwrap();

        let second_sid = session_storage_id("TST", &second.claims);
        let env_value = encode_session_env(&second_sid, &second_kp.to_seed_bytes());
        // SAFETY: env mutation serialized via ENV_LOCK
        unsafe { std::env::set_var("JOY_SESSION", &env_value) };

        let resolved = current_env_session("TST", "ai:claude@joy").unwrap();
        assert_eq!(
            resolved.claims.session_public_key, second.claims.session_public_key,
            "env resolves to the session the env points at, not the newest"
        );
        assert!(current_env_session("TST", "ai:other@joy").is_none());
        assert!(current_env_session("OTHER", "ai:claude@joy").is_none());

        // SAFETY: test cleanup
        unsafe { std::env::remove_var("JOY_SESSION") };
        assert!(current_env_session("TST", "ai:claude@joy").is_none());

        // SAFETY: test cleanup
        unsafe { std::env::remove_var("XDG_STATE_HOME") };
    }
}
