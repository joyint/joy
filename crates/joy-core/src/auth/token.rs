// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! AI delegation tokens with dual signatures (ADR-023, refined by ADR-033).
//!
//! Each token carries two Ed25519 signatures:
//! 1. Delegator signature (human's identity key) — proves authorization
//! 2. Binding signature (stable delegation key per (human, AI)) — binds to
//!    the public key recorded in `project.yaml` under
//!    `members[<human>].ai_delegations[<ai-member>].delegation_key`.
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
    /// Unique identifier for this specific token (UUID v4). Used to detect
    /// replay: once a token has been redeemed, subsequent redemption
    /// attempts for the same `token_id` are rejected (ADR-033).
    pub token_id: String,
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
    /// Hex-encoded Ed25519 signature by the stable delegation key.
    pub binding_signature: String,
    /// Hex-encoded public key of the delegation keypair. Redundant with the
    /// value recorded in `project.yaml` under `ai_delegations`; kept as an
    /// aid for debugging and for error messages pointing at a mismatch.
    pub delegation_public_key: String,
}

/// Create a delegation token with dual signatures.
///
/// The caller supplies the human's identity keypair (delegator, authorises
/// issuance via the first signature) and the stable per-(human, AI)
/// delegation keypair (produces the binding signature). The matching
/// `delegation_public_key` must already be recorded in `project.yaml`.
pub fn create_token(
    delegator_keypair: &IdentityKeypair,
    delegation_keypair: &IdentityKeypair,
    ai_member: &str,
    human: &str,
    project_id: &str,
    ttl: Option<Duration>,
) -> DelegationToken {
    let now = Utc::now();
    let claims = DelegationClaims {
        token_id: uuid::Uuid::new_v4().to_string(),
        ai_member: ai_member.to_string(),
        delegated_by: human.to_string(),
        project_id: project_id.to_string(),
        created: now,
        expires: ttl.map(|d| now + d),
    };
    let claims_json = serde_json::to_string(&claims).expect("claims serialize");

    let delegator_sig = delegator_keypair.sign(claims_json.as_bytes());
    let binding_sig = delegation_keypair.sign(claims_json.as_bytes());

    DelegationToken {
        claims,
        delegator_signature: hex::encode(delegator_sig),
        binding_signature: hex::encode(binding_sig),
        delegation_public_key: delegation_keypair.public_key().to_hex(),
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

    fn fresh_delegation() -> (sign::IdentityKeypair, sign::PublicKey) {
        let kp = sign::IdentityKeypair::from_random();
        let pk = kp.public_key();
        (kp, pk)
    }

    #[test]
    fn create_and_validate_token() {
        let (delegator, delegator_pk) = test_keypair();
        let (delegation, delegation_pk) = fresh_delegation();
        let token = create_token(
            &delegator,
            &delegation,
            "ai:claude@joy",
            "human@example.com",
            "TST",
            None,
        );
        let claims = validate_token(&token, &delegator_pk, &delegation_pk, "TST").unwrap();
        assert_eq!(claims.ai_member, "ai:claude@joy");
        assert_eq!(claims.delegated_by, "human@example.com");
        assert_eq!(token.delegation_public_key, delegation_pk.to_hex());
    }

    #[test]
    fn token_with_expiry() {
        let (delegator, delegator_pk) = test_keypair();
        let (delegation, delegation_pk) = fresh_delegation();
        let token = create_token(
            &delegator,
            &delegation,
            "ai:claude@joy",
            "human@example.com",
            "TST",
            Some(Duration::hours(8)),
        );
        let claims = validate_token(&token, &delegator_pk, &delegation_pk, "TST").unwrap();
        assert!(claims.expires.is_some());
    }

    #[test]
    fn expired_token_rejected() {
        let (delegator, delegator_pk) = test_keypair();
        let (delegation, delegation_pk) = fresh_delegation();
        let token = create_token(
            &delegator,
            &delegation,
            "ai:claude@joy",
            "human@example.com",
            "TST",
            Some(Duration::seconds(-1)),
        );
        assert!(validate_token(&token, &delegator_pk, &delegation_pk, "TST").is_err());
    }

    #[test]
    fn wrong_project_rejected() {
        let (delegator, delegator_pk) = test_keypair();
        let (delegation, delegation_pk) = fresh_delegation();
        let token = create_token(
            &delegator,
            &delegation,
            "ai:claude@joy",
            "human@example.com",
            "TST",
            None,
        );
        assert!(validate_token(&token, &delegator_pk, &delegation_pk, "OTHER").is_err());
    }

    #[test]
    fn tampered_claims_rejected() {
        let (delegator, delegator_pk) = test_keypair();
        let (delegation, delegation_pk) = fresh_delegation();
        let mut token = create_token(
            &delegator,
            &delegation,
            "ai:claude@joy",
            "human@example.com",
            "TST",
            None,
        );
        token.claims.ai_member = "ai:attacker@evil".into();
        assert!(validate_token(&token, &delegator_pk, &delegation_pk, "TST").is_err());
    }

    #[test]
    fn wrong_delegator_key_rejected() {
        let (delegator, _) = test_keypair();
        let (delegation, delegation_pk) = fresh_delegation();
        let token = create_token(
            &delegator,
            &delegation,
            "ai:claude@joy",
            "human@example.com",
            "TST",
            None,
        );

        let other_salt = derive::generate_salt();
        let other_key =
            derive::derive_key("alpha bravo charlie delta echo foxtrot", &other_salt).unwrap();
        let other_kp = sign::IdentityKeypair::from_derived_key(&other_key);
        let other_pk = other_kp.public_key();

        assert!(validate_token(&token, &other_pk, &delegation_pk, "TST").is_err());
    }

    #[test]
    fn wrong_delegation_key_rejected() {
        let (delegator, delegator_pk) = test_keypair();
        let (delegation, _) = fresh_delegation();
        let token = create_token(
            &delegator,
            &delegation,
            "ai:claude@joy",
            "human@example.com",
            "TST",
            None,
        );

        // Simulates rotation: validator looks up a different delegation_key in project.yaml.
        let (_, rotated_pk) = fresh_delegation();
        assert!(validate_token(&token, &delegator_pk, &rotated_pk, "TST").is_err());
    }

    #[test]
    fn encode_decode_roundtrip() {
        let (delegator, delegator_pk) = test_keypair();
        let (delegation, delegation_pk) = fresh_delegation();
        let token = create_token(
            &delegator,
            &delegation,
            "ai:claude@joy",
            "human@example.com",
            "TST",
            None,
        );
        let encoded = encode_token(&token);
        assert!(encoded.starts_with("joy_t_"));
        let decoded = decode_token(&encoded).unwrap();
        let claims = validate_token(&decoded, &delegator_pk, &delegation_pk, "TST").unwrap();
        assert_eq!(claims.ai_member, "ai:claude@joy");
    }

    #[test]
    fn invalid_prefix_rejected() {
        assert!(decode_token("invalid_prefix_data").is_err());
    }
}
