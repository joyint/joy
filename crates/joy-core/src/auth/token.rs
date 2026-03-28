// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Per-delegator AI delegation tokens with dual signatures (ADR-023).
//!
//! Each token carries two Ed25519 signatures:
//! 1. Delegator signature (human's identity key) — proves authorization
//! 2. Token binding signature (one-time token_key) — binds to project.yaml entry
//!
//! Tokens are passed via `--token` flag or `JOY_TOKEN` env var to `joy auth`.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use super::sign::{IdentityKeypair, PublicKey};
use crate::error::JoyError;

/// Token prefix for visual identification.
const TOKEN_PREFIX: &str = "joy_t_";

/// Claims encoded in a delegation token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationClaims {
    pub ai_member: String,
    pub delegated_by: String,
    pub project_id: String,
    pub created: DateTime<Utc>,
    pub expires: Option<DateTime<Utc>>,
}

/// A delegation token with dual signatures.
#[derive(Debug, Serialize, Deserialize)]
pub struct DelegationToken {
    pub claims: DelegationClaims,
    /// Hex-encoded Ed25519 signature by the delegating human's key.
    pub delegator_signature: String,
    /// Hex-encoded Ed25519 signature by the one-time token_key.
    pub binding_signature: String,
    /// Hex-encoded public key of the one-time token_key (for matching against project.yaml).
    pub token_public_key: String,
}

/// Result of creating a token: the encoded token string + the public key to store.
pub struct CreateTokenResult {
    pub token: DelegationToken,
    pub token_public_key: String,
}

/// Create a delegation token with dual signatures.
///
/// Returns the token and the token_key public key (to store in project.yaml).
pub fn create_token(
    delegator_keypair: &IdentityKeypair,
    ai_member: &str,
    human: &str,
    project_id: &str,
    ttl: Option<Duration>,
) -> CreateTokenResult {
    let now = Utc::now();
    let claims = DelegationClaims {
        ai_member: ai_member.to_string(),
        delegated_by: human.to_string(),
        project_id: project_id.to_string(),
        created: now,
        expires: ttl.map(|d| now + d),
    };
    let claims_json = serde_json::to_string(&claims).expect("claims serialize");

    // Signature 1: delegator's identity key
    let delegator_sig = delegator_keypair.sign(claims_json.as_bytes());

    // Signature 2: one-time token key (generated fresh)
    let token_keypair = IdentityKeypair::from_random();
    let binding_sig = token_keypair.sign(claims_json.as_bytes());
    let token_pk = token_keypair.public_key();

    CreateTokenResult {
        token: DelegationToken {
            claims,
            delegator_signature: hex::encode(delegator_sig),
            binding_signature: hex::encode(binding_sig),
            token_public_key: token_pk.to_hex(),
        },
        token_public_key: token_pk.to_hex(),
    }
}

/// Validate a delegation token against both the delegator's key and the token_key.
pub fn validate_token(
    token: &DelegationToken,
    delegator_pk: &PublicKey,
    token_pk: &PublicKey,
    project_id: &str,
) -> Result<DelegationClaims, JoyError> {
    // Check project match
    if token.claims.project_id != project_id {
        return Err(JoyError::AuthFailed(
            "token belongs to a different project".into(),
        ));
    }

    // Check expiry
    if let Some(expires) = token.claims.expires {
        if Utc::now() > expires {
            return Err(JoyError::AuthFailed("delegation token expired".into()));
        }
    }

    let claims_json = serde_json::to_string(&token.claims).expect("claims serialize");

    // Verify delegator signature
    let delegator_sig = hex::decode(&token.delegator_signature)
        .map_err(|e| JoyError::AuthFailed(format!("{e}")))?;
    delegator_pk.verify(claims_json.as_bytes(), &delegator_sig)?;

    // Verify binding signature
    let binding_sig =
        hex::decode(&token.binding_signature).map_err(|e| JoyError::AuthFailed(format!("{e}")))?;
    token_pk.verify(claims_json.as_bytes(), &binding_sig)?;

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
    use crate::auth::{derive, sign};
    use chrono::Duration;

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
    fn create_and_validate_token() {
        let (kp, pk) = test_keypair();
        let result = create_token(&kp, "ai:claude@joy", "human@example.com", "TST", None);
        let token_pk = sign::PublicKey::from_hex(&result.token_public_key).unwrap();
        let claims = validate_token(&result.token, &pk, &token_pk, "TST").unwrap();
        assert_eq!(claims.ai_member, "ai:claude@joy");
        assert_eq!(claims.delegated_by, "human@example.com");
    }

    #[test]
    fn token_with_expiry() {
        let (kp, pk) = test_keypair();
        let result = create_token(
            &kp,
            "ai:claude@joy",
            "human@example.com",
            "TST",
            Some(Duration::hours(8)),
        );
        let token_pk = sign::PublicKey::from_hex(&result.token_public_key).unwrap();
        let claims = validate_token(&result.token, &pk, &token_pk, "TST").unwrap();
        assert!(claims.expires.is_some());
    }

    #[test]
    fn expired_token_rejected() {
        let (kp, pk) = test_keypair();
        let result = create_token(
            &kp,
            "ai:claude@joy",
            "human@example.com",
            "TST",
            Some(Duration::seconds(-1)),
        );
        let token_pk = sign::PublicKey::from_hex(&result.token_public_key).unwrap();
        assert!(validate_token(&result.token, &pk, &token_pk, "TST").is_err());
    }

    #[test]
    fn wrong_project_rejected() {
        let (kp, pk) = test_keypair();
        let result = create_token(&kp, "ai:claude@joy", "human@example.com", "TST", None);
        let token_pk = sign::PublicKey::from_hex(&result.token_public_key).unwrap();
        assert!(validate_token(&result.token, &pk, &token_pk, "OTHER").is_err());
    }

    #[test]
    fn tampered_claims_rejected() {
        let (kp, pk) = test_keypair();
        let result = create_token(&kp, "ai:claude@joy", "human@example.com", "TST", None);
        let token_pk = sign::PublicKey::from_hex(&result.token_public_key).unwrap();
        let mut token = result.token;
        token.claims.ai_member = "ai:attacker@evil".into();
        assert!(validate_token(&token, &pk, &token_pk, "TST").is_err());
    }

    #[test]
    fn wrong_delegator_key_rejected() {
        let (kp, _) = test_keypair();
        let result = create_token(&kp, "ai:claude@joy", "human@example.com", "TST", None);
        let token_pk = sign::PublicKey::from_hex(&result.token_public_key).unwrap();

        // Different delegator key
        let other_salt = derive::generate_salt();
        let other_key =
            derive::derive_key("alpha bravo charlie delta echo foxtrot", &other_salt).unwrap();
        let other_kp = sign::IdentityKeypair::from_derived_key(&other_key);
        let other_pk = other_kp.public_key();

        assert!(validate_token(&result.token, &other_pk, &token_pk, "TST").is_err());
    }

    #[test]
    fn wrong_token_key_rejected() {
        let (kp, pk) = test_keypair();
        let result = create_token(&kp, "ai:claude@joy", "human@example.com", "TST", None);

        // Different token key (simulates revoked token)
        let wrong_token_kp = sign::IdentityKeypair::from_random();
        let wrong_token_pk = wrong_token_kp.public_key();

        assert!(validate_token(&result.token, &pk, &wrong_token_pk, "TST").is_err());
    }

    #[test]
    fn encode_decode_roundtrip() {
        let (kp, pk) = test_keypair();
        let result = create_token(&kp, "ai:claude@joy", "human@example.com", "TST", None);
        let encoded = encode_token(&result.token);
        assert!(encoded.starts_with("joy_t_"));
        let decoded = decode_token(&encoded).unwrap();
        let token_pk = sign::PublicKey::from_hex(&result.token_public_key).unwrap();
        let claims = validate_token(&decoded, &pk, &token_pk, "TST").unwrap();
        assert_eq!(claims.ai_member, "ai:claude@joy");
    }

    #[test]
    fn invalid_prefix_rejected() {
        assert!(decode_token("invalid_prefix_data").is_err());
    }
}
