// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Delegation tokens for AI members.
//!
//! A delegation token is created by an authenticated human and given to an
//! AI agent. It proves that the human authorized the AI to act on their behalf.
//! The token is signed with the human's Ed25519 identity key.
//!
//! Tokens are passed via `JOY_AUTH_TOKEN` environment variable.

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

/// A delegation token: claims + Ed25519 signature.
#[derive(Debug, Serialize, Deserialize)]
pub struct DelegationToken {
    pub claims: DelegationClaims,
    pub signature: String,
}

/// Create a delegation token signed by the human's identity keypair.
pub fn create_token(
    keypair: &IdentityKeypair,
    ai_member: &str,
    human: &str,
    project_id: &str,
    ttl: Option<Duration>,
) -> DelegationToken {
    let now = Utc::now();
    let claims = DelegationClaims {
        ai_member: ai_member.to_string(),
        delegated_by: human.to_string(),
        project_id: project_id.to_string(),
        created: now,
        expires: ttl.map(|d| now + d),
    };
    let claims_json = serde_json::to_string(&claims).expect("claims serialize");
    let signature = keypair.sign(claims_json.as_bytes());
    DelegationToken {
        claims,
        signature: hex::encode(signature),
    }
}

/// Validate a delegation token against the delegating human's public key.
pub fn validate_token(
    token: &DelegationToken,
    public_key: &PublicKey,
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

    // Verify signature
    let claims_json = serde_json::to_string(&token.claims).expect("claims serialize");
    let signature =
        hex::decode(&token.signature).map_err(|e| JoyError::AuthFailed(format!("{e}")))?;
    public_key.verify(claims_json.as_bytes(), &signature)?;

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

// Simple base64 encoding/decoding using standard alphabet
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
        let token = create_token(&kp, "ai:claude@joy", "human@example.com", "TST", None);
        let claims = validate_token(&token, &pk, "TST").unwrap();
        assert_eq!(claims.ai_member, "ai:claude@joy");
        assert_eq!(claims.delegated_by, "human@example.com");
        assert_eq!(claims.project_id, "TST");
        assert!(claims.expires.is_none());
    }

    #[test]
    fn token_with_expiry() {
        let (kp, pk) = test_keypair();
        let token = create_token(
            &kp,
            "ai:claude@joy",
            "human@example.com",
            "TST",
            Some(Duration::hours(8)),
        );
        let claims = validate_token(&token, &pk, "TST").unwrap();
        assert!(claims.expires.is_some());
    }

    #[test]
    fn expired_token_rejected() {
        let (kp, pk) = test_keypair();
        let token = create_token(
            &kp,
            "ai:claude@joy",
            "human@example.com",
            "TST",
            Some(Duration::seconds(-1)),
        );
        assert!(validate_token(&token, &pk, "TST").is_err());
    }

    #[test]
    fn wrong_project_rejected() {
        let (kp, pk) = test_keypair();
        let token = create_token(&kp, "ai:claude@joy", "human@example.com", "TST", None);
        assert!(validate_token(&token, &pk, "OTHER").is_err());
    }

    #[test]
    fn tampered_token_rejected() {
        let (kp, pk) = test_keypair();
        let mut token = create_token(&kp, "ai:claude@joy", "human@example.com", "TST", None);
        token.claims.ai_member = "ai:attacker@evil".into();
        assert!(validate_token(&token, &pk, "TST").is_err());
    }

    #[test]
    fn encode_decode_roundtrip() {
        let (kp, pk) = test_keypair();
        let token = create_token(&kp, "ai:claude@joy", "human@example.com", "TST", None);
        let encoded = encode_token(&token);
        assert!(encoded.starts_with("joy_t_"));
        let decoded = decode_token(&encoded).unwrap();
        let claims = validate_token(&decoded, &pk, "TST").unwrap();
        assert_eq!(claims.ai_member, "ai:claude@joy");
    }

    #[test]
    fn invalid_prefix_rejected() {
        assert!(decode_token("invalid_prefix_data").is_err());
    }

    #[test]
    fn wrong_key_rejected() {
        let (kp, _) = test_keypair();
        let token = create_token(&kp, "ai:claude@joy", "human@example.com", "TST", None);

        let other_salt = derive::generate_salt();
        let other_key =
            derive::derive_key("alpha bravo charlie delta echo foxtrot", &other_salt).unwrap();
        let other_kp = sign::IdentityKeypair::from_derived_key(&other_key);
        let other_pk = other_kp.public_key();

        assert!(validate_token(&token, &other_pk, "TST").is_err());
    }
}
