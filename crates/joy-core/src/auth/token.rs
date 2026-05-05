// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! AI delegation tokens with dual signatures (ADR-023, refined by ADR-033 and
//! ADR-041).
//!
//! Each token carries two Ed25519 signatures:
//! 1. Delegator signature (human's identity key) — proves authorization
//! 2. Binding signature (stable delegation key per (human, AI)) — binds to
//!    the public key recorded in `project.yaml` under
//!    `members[<human>].ai_delegations[<ai-member>].delegation_verifier`.
//!
//! Tokens carry a `scopes` claim (ADR-041 §3). The default `["auth"]` lets
//! the AI run joy commands as the AI member. With `--crypt` (`["auth",
//! "crypt"]`) the token additionally embeds the delegation private key as
//! a 32-byte Ed25519 seed so the AI can unwrap zone keys for the duration
//! of the token's TTL.
//!
//! Tokens are passed via `--token` flag or `JOY_TOKEN` env var to `joy auth`.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use super::{IdentityKeypair, PublicKey};
use crate::error::JoyError;

/// Token prefix for visual identification.
const TOKEN_PREFIX: &str = "joy_t_";

/// Default scope set when a token's claims omit the field (back-compat).
fn default_scopes() -> Vec<String> {
    vec!["auth".to_string()]
}

/// Scope value indicating the token additionally carries the delegation
/// private key for Crypt unwrap (ADR-041).
pub const SCOPE_CRYPT: &str = "crypt";
/// Scope value for ordinary AI command authentication (default).
pub const SCOPE_AUTH: &str = "auth";

/// Claims encoded in a delegation token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationClaims {
    /// Unique identifier for this specific token (UUID v4). Used to detect
    /// replay: once a token has been redeemed, subsequent redemption
    /// attempts for the same `token_id` are rejected (ADR-033).
    pub token_id: String,
    pub ai_member: String,
    pub delegated_by: String,
    pub project_id: String,
    pub created: DateTime<Utc>,
    pub expires: Option<DateTime<Utc>>,
    /// Capability scopes for this token. Default `["auth"]`. Tokens issued
    /// with `--crypt` also carry `"crypt"` (ADR-041 §3). Unknown scopes are
    /// preserved so newer scope vocabularies do not break older verifiers.
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,
}

impl DelegationClaims {
    /// Whether the token authorises Crypt unwrap (carries delegation privkey).
    pub fn has_crypt_scope(&self) -> bool {
        self.scopes.iter().any(|s| s == SCOPE_CRYPT)
    }
}

/// A delegation token with dual signatures.
#[derive(Debug, Serialize, Deserialize)]
pub struct DelegationToken {
    pub claims: DelegationClaims,
    /// Hex-encoded Ed25519 signature by the delegating human's key.
    pub delegator_signature: String,
    /// Hex-encoded Ed25519 signature by the stable delegation key.
    pub binding_signature: String,
    /// Hex-encoded public key of the delegation keypair. Redundant with the
    /// value recorded in `project.yaml` under `ai_delegations`; kept as an
    /// aid for debugging and for error messages pointing at a mismatch.
    pub delegation_public_key: String,
    /// Hex-encoded 32-byte Ed25519 seed for the delegation keypair. Present
    /// only on tokens with `crypt` scope; lets the AI re-derive the
    /// delegation keypair to unwrap zone keys (ADR-041 §2). Never persisted
    /// on the AI's disk; it travels in the token string and lives in the
    /// `JOY_SESSION` env var while a session is active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation_private_key: Option<String>,
}

/// Cryptographic material used to sign a delegation token.
pub struct TokenSigningKeys<'a> {
    /// Human's identity keypair, produces the delegator signature.
    pub delegator: &'a IdentityKeypair,
    /// Stable per-(human, AI) delegation keypair, produces the binding
    /// signature. The matching public key must already be recorded in
    /// `project.yaml`.
    pub delegation: &'a IdentityKeypair,
    /// 32-byte Ed25519 seed of the delegation keypair. Embedded in the
    /// token wire format when [`TokenIssueParams::crypt_scope`] is true
    /// (ADR-041 §3); otherwise discarded.
    pub delegation_seed: &'a [u8; 32],
}

/// Identity and policy fields for a token issuance.
pub struct TokenIssueParams<'a> {
    pub ai_member: &'a str,
    pub human: &'a str,
    pub project_id: &'a str,
    pub ttl: Option<Duration>,
    /// Whether the token also carries the delegation private key for
    /// Crypt unwrap (ADR-041 §3). When false the token is auth-only.
    pub crypt_scope: bool,
}

/// Create a delegation token with dual signatures.
///
/// The caller supplies the signing material (delegator + delegation
/// keypair, plus the delegation seed for Crypt-scope tokens) and the
/// claim parameters. When `params.crypt_scope` is true, the delegation
/// seed is embedded in the token so the AI can re-derive the keypair
/// and unwrap zone keys; the scope claim becomes `["auth", "crypt"]`.
/// When false, the seed is discarded and the token is auth-only.
pub fn create_token(keys: TokenSigningKeys<'_>, params: TokenIssueParams<'_>) -> DelegationToken {
    let now = Utc::now();
    let scopes = if params.crypt_scope {
        vec![SCOPE_AUTH.to_string(), SCOPE_CRYPT.to_string()]
    } else {
        vec![SCOPE_AUTH.to_string()]
    };
    let claims = DelegationClaims {
        token_id: uuid::Uuid::new_v4().to_string(),
        ai_member: params.ai_member.to_string(),
        delegated_by: params.human.to_string(),
        project_id: params.project_id.to_string(),
        created: now,
        expires: params.ttl.map(|d| now + d),
        scopes,
    };
    let claims_json = serde_json::to_string(&claims).expect("claims serialize");

    let delegator_sig = keys.delegator.sign(claims_json.as_bytes());
    let binding_sig = keys.delegation.sign(claims_json.as_bytes());

    let delegation_private_key = if params.crypt_scope {
        Some(hex::encode(keys.delegation_seed))
    } else {
        None
    };

    DelegationToken {
        claims,
        delegator_signature: hex::encode(delegator_sig),
        binding_signature: hex::encode(binding_sig),
        delegation_public_key: keys.delegation.public_key().to_hex(),
        delegation_private_key,
    }
}

/// Validate a delegation token against the delegator's identity key and the
/// stable delegation key recorded in `project.yaml`.
pub fn validate_token(
    token: &DelegationToken,
    delegator_pk: &PublicKey,
    delegation_pk: &PublicKey,
    project_id: &str,
) -> Result<DelegationClaims, JoyError> {
    if token.claims.project_id != project_id {
        return Err(JoyError::AuthFailed(
            "token belongs to a different project".into(),
        ));
    }

    if let Some(expires) = token.claims.expires {
        if Utc::now() > expires {
            return Err(JoyError::AuthFailed(format!(
                "Token expired (issued {}, expired {}). \
                 Ask the human to issue a new one with: joy auth token add {}",
                token.claims.created.format("%Y-%m-%d %H:%M UTC"),
                expires.format("%Y-%m-%d %H:%M UTC"),
                token.claims.ai_member
            )));
        }
    }

    let claims_json = serde_json::to_string(&token.claims).expect("claims serialize");

    let delegator_sig = hex::decode(&token.delegator_signature)
        .map_err(|e| JoyError::AuthFailed(format!("{e}")))?;
    delegator_pk.verify(claims_json.as_bytes(), &delegator_sig)?;

    let binding_sig =
        hex::decode(&token.binding_signature).map_err(|e| JoyError::AuthFailed(format!("{e}")))?;
    delegation_pk.verify(claims_json.as_bytes(), &binding_sig)?;

    Ok(token.claims.clone())
}

/// Encode a token as a portable string (`joy_t_<base64>`).
pub fn encode_token(token: &DelegationToken) -> String {
    let json = serde_json::to_string(token).expect("token serialize");
    let encoded = base64_encode(json.as_bytes());
    format!("{TOKEN_PREFIX}{encoded}")
}

/// Decode a token from its portable string representation.
pub fn decode_token(s: &str) -> Result<DelegationToken, JoyError> {
    let data = s.strip_prefix(TOKEN_PREFIX).ok_or_else(|| {
        JoyError::AuthFailed("invalid token format (missing joy_t_ prefix)".into())
    })?;
    let json = base64_decode(data)?;
    let token: DelegationToken = serde_json::from_slice(&json)
        .map_err(|e| JoyError::AuthFailed(format!("invalid token: {e}")))?;
    Ok(token)
}

/// Check if a string looks like a delegation token (has the `joy_t_` prefix).
pub fn is_token(s: &str) -> bool {
    s.starts_with(TOKEN_PREFIX)
}

fn base64_encode(data: &[u8]) -> String {
    use base64ct::{Base64, Encoding};
    Base64::encode_string(data)
}

fn base64_decode(s: &str) -> Result<Vec<u8>, JoyError> {
    use base64ct::{Base64, Encoding};
    Base64::decode_vec(s).map_err(|e| JoyError::AuthFailed(format!("base64 decode: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{derive_key, Salt};
    use chrono::Duration;

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

    fn fresh_delegation() -> ([u8; 32], IdentityKeypair, PublicKey) {
        use rand::RngCore;
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        let kp = IdentityKeypair::from_seed(&seed);
        let pk = kp.public_key();
        (seed, kp, pk)
    }

    fn make_token(
        delegator: &IdentityKeypair,
        delegation: &IdentityKeypair,
        seed: &[u8; 32],
        ttl: Option<Duration>,
        crypt_scope: bool,
    ) -> DelegationToken {
        create_token(
            TokenSigningKeys {
                delegator,
                delegation,
                delegation_seed: seed,
            },
            TokenIssueParams {
                ai_member: "ai:claude@joy",
                human: "human@example.com",
                project_id: "TST",
                ttl,
                crypt_scope,
            },
        )
    }

    #[test]
    fn create_and_validate_token() {
        let (delegator, delegator_pk) = test_keypair();
        let (seed, delegation, delegation_pk) = fresh_delegation();
        let token = make_token(&delegator, &delegation, &seed, None, false);
        let claims = validate_token(&token, &delegator_pk, &delegation_pk, "TST").unwrap();
        assert_eq!(claims.ai_member, "ai:claude@joy");
        assert_eq!(claims.delegated_by, "human@example.com");
        assert_eq!(token.delegation_public_key, delegation_pk.to_hex());
        assert!(token.delegation_private_key.is_none());
        assert!(!claims.has_crypt_scope());
    }

    #[test]
    fn crypt_token_carries_seed() {
        let (delegator, _) = test_keypair();
        let (seed, delegation, _) = fresh_delegation();
        let token = make_token(&delegator, &delegation, &seed, None, true);
        assert_eq!(token.delegation_private_key, Some(hex::encode(seed)));
        assert!(token.claims.has_crypt_scope());
    }

    #[test]
    fn token_with_expiry() {
        let (delegator, delegator_pk) = test_keypair();
        let (seed, delegation, delegation_pk) = fresh_delegation();
        let token = make_token(
            &delegator,
            &delegation,
            &seed,
            Some(Duration::hours(8)),
            false,
        );
        let claims = validate_token(&token, &delegator_pk, &delegation_pk, "TST").unwrap();
        assert!(claims.expires.is_some());
    }

    #[test]
    fn expired_token_rejected() {
        let (delegator, delegator_pk) = test_keypair();
        let (seed, delegation, delegation_pk) = fresh_delegation();
        let token = make_token(
            &delegator,
            &delegation,
            &seed,
            Some(Duration::seconds(-1)),
            false,
        );
        assert!(validate_token(&token, &delegator_pk, &delegation_pk, "TST").is_err());
    }

    #[test]
    fn wrong_project_rejected() {
        let (delegator, delegator_pk) = test_keypair();
        let (seed, delegation, delegation_pk) = fresh_delegation();
        let token = make_token(&delegator, &delegation, &seed, None, false);
        assert!(validate_token(&token, &delegator_pk, &delegation_pk, "OTHER").is_err());
    }

    #[test]
    fn tampered_claims_rejected() {
        let (delegator, delegator_pk) = test_keypair();
        let (seed, delegation, delegation_pk) = fresh_delegation();
        let mut token = make_token(&delegator, &delegation, &seed, None, false);
        token.claims.ai_member = "ai:attacker@evil".into();
        assert!(validate_token(&token, &delegator_pk, &delegation_pk, "TST").is_err());
    }

    #[test]
    fn wrong_delegator_key_rejected() {
        let (delegator, _) = test_keypair();
        let (seed, delegation, delegation_pk) = fresh_delegation();
        let token = make_token(&delegator, &delegation, &seed, None, false);

        let other_salt = crate::auth::generate_salt();
        let other_key = derive_key("alpha bravo charlie delta echo foxtrot", &other_salt).unwrap();
        let other_kp = IdentityKeypair::from_derived_key(&other_key);
        let other_pk = other_kp.public_key();

        assert!(validate_token(&token, &other_pk, &delegation_pk, "TST").is_err());
    }

    #[test]
    fn wrong_delegation_key_rejected() {
        let (delegator, delegator_pk) = test_keypair();
        let (seed, delegation, _) = fresh_delegation();
        let token = make_token(&delegator, &delegation, &seed, None, false);

        // Simulates rotation: validator looks up a different delegation_key in project.yaml.
        let (_, _, rotated_pk) = fresh_delegation();
        assert!(validate_token(&token, &delegator_pk, &rotated_pk, "TST").is_err());
    }

    #[test]
    fn encode_decode_roundtrip() {
        let (delegator, delegator_pk) = test_keypair();
        let (seed, delegation, delegation_pk) = fresh_delegation();
        let token = make_token(&delegator, &delegation, &seed, None, false);
        let encoded = encode_token(&token);
        assert!(encoded.starts_with("joy_t_"));
        let decoded = decode_token(&encoded).unwrap();
        let claims = validate_token(&decoded, &delegator_pk, &delegation_pk, "TST").unwrap();
        assert_eq!(claims.ai_member, "ai:claude@joy");
    }

    #[test]
    fn legacy_token_without_scopes_field_decodes() {
        // Old tokens (pre-ADR-041) had no `scopes` field in claims. Ensure
        // they still deserialize, with the default scope ["auth"].
        let legacy_json = r#"{
            "claims": {
                "token_id": "abc",
                "ai_member": "ai:claude@joy",
                "delegated_by": "human@example.com",
                "project_id": "TST",
                "created": "2026-05-01T00:00:00Z",
                "expires": null
            },
            "delegator_signature": "00",
            "binding_signature": "00",
            "delegation_public_key": "00"
        }"#;
        let token: DelegationToken = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(token.claims.scopes, vec!["auth".to_string()]);
        assert!(!token.claims.has_crypt_scope());
        assert!(token.delegation_private_key.is_none());
    }

    #[test]
    fn invalid_prefix_rejected() {
        assert!(decode_token("invalid_prefix_data").is_err());
    }
}
