// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Argon2id key derivation from passphrase and project-bound salt.
//!
//! Produces 32 bytes of key material suitable for Ed25519 seed generation.
//! Parameters match Bitwarden defaults: 64 MiB memory, 3 iterations, 4 lanes.
//!
//! ADR-037 adds a second derivation: HKDF-SHA256 over the human's identity
//! material plus a per-(human, AI) salt yields the deterministic delegation
//! seed used for Ed25519 delegation keypairs. See `derive_delegation_seed`.

use argon2::{Algorithm, Argon2, Params, Version};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::Sha256;
use zeroize::{Zeroize, Zeroizing};

use crate::error::JoyError;

/// Random 32-byte salt, stored per-member in project.yaml as hex.
#[derive(Clone)]
pub struct Salt([u8; 32]);

impl Salt {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self, JoyError> {
        let bytes =
            hex::decode(s).map_err(|e| JoyError::AuthFailed(format!("invalid salt: {e}")))?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| JoyError::AuthFailed("salt must be 32 bytes".into()))?;
        Ok(Self(arr))
    }
}

/// 32-byte derived key material. Zeroed on drop.
pub struct DerivedKey(Zeroizing<[u8; 32]>);

impl DerivedKey {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Generate a random 32-byte salt.
pub fn generate_salt() -> Salt {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    Salt(bytes)
}

/// Derive 32 bytes of key material from a passphrase and salt using Argon2id.
///
/// Production parameters: m_cost=65536 (64 MiB), t_cost=3, p_cost=4, output=32 bytes.
/// Debug builds and `fast-kdf` feature: m_cost=256, t_cost=1, p_cost=1 (fast, NOT secure).
pub fn derive_key(passphrase: &str, salt: &Salt) -> Result<DerivedKey, JoyError> {
    #[cfg(any(feature = "fast-kdf", debug_assertions))]
    let params = Params::new(256, 1, 1, Some(32))
        .map_err(|e| JoyError::AuthFailed(format!("argon2 params: {e}")))?;
    #[cfg(not(any(feature = "fast-kdf", debug_assertions)))]
    let params = Params::new(65536, 3, 4, Some(32))
        .map_err(|e| JoyError::AuthFailed(format!("argon2 params: {e}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let mut output = Zeroizing::new([0u8; 32]);
    argon2
        .hash_password_into(passphrase.as_bytes(), salt.as_bytes(), output.as_mut())
        .map_err(|e| JoyError::AuthFailed(format!("key derivation failed: {e}")))?;

    Ok(DerivedKey(output))
}

/// Derive a 32-byte Ed25519 seed for a per-(human, AI) delegation key from
/// the human's Argon2id-derived identity material plus a per-(human, AI)
/// salt (ADR-037).
///
/// Inputs:
///   - `identity_key`: the 32-byte Argon2id output for the human (already in
///     hand from `derive_key(passphrase, kdf_nonce)`).
///   - `salt`: the per-(human, AI) `delegation_salt` recorded in
///     `project.yaml`.
///   - `project_id`: the canonical project id (acronym today).
///   - `ai_member_id`: the AI member id, e.g. `ai:claude@joy`.
///
/// Output: 32 bytes suitable for `IdentityKeypair::from_seed`.
///
/// HKDF-SHA256 is used in Extract-and-Expand form. The `info` parameter
/// embeds project and member ids so the same `(identity_key, salt)` cannot
/// be replayed across (project, AI) pairs.
pub fn derive_delegation_seed(
    identity_key: &DerivedKey,
    salt: &Salt,
    project_id: &str,
    ai_member_id: &str,
) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(salt.as_bytes()), identity_key.as_bytes());
    let mut info = Vec::with_capacity(16 + project_id.len() + 1 + ai_member_id.len());
    info.extend_from_slice(b"joy-delegation:");
    info.extend_from_slice(project_id.as_bytes());
    info.push(b':');
    info.extend_from_slice(ai_member_id.as_bytes());
    let mut out = [0u8; 32];
    hk.expand(&info, &mut out)
        .expect("HKDF-SHA256 expand to 32 bytes never fails");
    out
}

/// Validate that a passphrase has at least 6 whitespace-separated words.
pub fn validate_passphrase(passphrase: &str) -> Result<(), JoyError> {
    let word_count = passphrase.split_whitespace().count();
    if word_count < 6 {
        return Err(JoyError::PassphraseTooShort);
    }
    Ok(())
}

impl Drop for Salt {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PASSPHRASE: &str = "correct horse battery staple extra words";

    #[test]
    fn salt_is_random() {
        let s1 = generate_salt();
        let s2 = generate_salt();
        assert_ne!(s1.as_bytes(), s2.as_bytes());
    }

    #[test]
    fn salt_hex_roundtrip() {
        let salt = generate_salt();
        let hex = salt.to_hex();
        let parsed = Salt::from_hex(&hex).unwrap();
        assert_eq!(salt.as_bytes(), parsed.as_bytes());
    }

    #[test]
    fn derive_deterministic() {
        let salt =
            Salt::from_hex("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .unwrap();
        let k1 = derive_key(TEST_PASSPHRASE, &salt).unwrap();
        let k2 = derive_key(TEST_PASSPHRASE, &salt).unwrap();
        assert_eq!(k1.as_bytes(), k2.as_bytes());
    }

    #[test]
    fn derive_different_salt() {
        let s1 = generate_salt();
        let s2 = generate_salt();
        let k1 = derive_key(TEST_PASSPHRASE, &s1).unwrap();
        let k2 = derive_key(TEST_PASSPHRASE, &s2).unwrap();
        assert_ne!(k1.as_bytes(), k2.as_bytes());
    }

    #[test]
    fn derive_different_passphrase() {
        let salt = generate_salt();
        let k1 = derive_key("one two three four five six", &salt).unwrap();
        let k2 = derive_key("seven eight nine ten eleven twelve", &salt).unwrap();
        assert_ne!(k1.as_bytes(), k2.as_bytes());
    }

    #[test]
    fn passphrase_too_short() {
        assert!(validate_passphrase("one two three").is_err());
        assert!(validate_passphrase("one two three four five").is_err());
    }

    #[test]
    fn passphrase_valid() {
        assert!(validate_passphrase("one two three four five six").is_ok());
        assert!(validate_passphrase("a b c d e f g h").is_ok());
    }

    /// Helper that mints a deterministic identity key from a fixed salt so
    /// delegation-seed tests do not depend on Argon2id parameters.
    fn fixed_identity_key() -> DerivedKey {
        let bytes = Zeroizing::new([7u8; 32]);
        DerivedKey(bytes)
    }

    #[test]
    fn delegation_seed_is_deterministic() {
        let salt = generate_salt();
        let id = fixed_identity_key();
        let s1 = derive_delegation_seed(&id, &salt, "JOY", "ai:claude@joy");
        let s2 = derive_delegation_seed(&id, &salt, "JOY", "ai:claude@joy");
        assert_eq!(s1, s2);
    }

    #[test]
    fn delegation_seed_changes_with_salt() {
        let id = fixed_identity_key();
        let s1 = derive_delegation_seed(&id, &generate_salt(), "JOY", "ai:claude@joy");
        let s2 = derive_delegation_seed(&id, &generate_salt(), "JOY", "ai:claude@joy");
        assert_ne!(s1, s2);
    }

    #[test]
    fn delegation_seed_is_domain_separated_by_project() {
        let salt = generate_salt();
        let id = fixed_identity_key();
        let s1 = derive_delegation_seed(&id, &salt, "JOY", "ai:claude@joy");
        let s2 = derive_delegation_seed(&id, &salt, "OTHER", "ai:claude@joy");
        assert_ne!(s1, s2);
    }

    #[test]
    fn delegation_seed_is_domain_separated_by_member() {
        let salt = generate_salt();
        let id = fixed_identity_key();
        let s1 = derive_delegation_seed(&id, &salt, "JOY", "ai:claude@joy");
        let s2 = derive_delegation_seed(&id, &salt, "JOY", "ai:qwen@joy");
        assert_ne!(s1, s2);
    }

    #[test]
    fn delegation_seed_changes_with_identity_key() {
        let salt = generate_salt();
        let id_a = fixed_identity_key();
        let id_b = {
            let bytes = Zeroizing::new([8u8; 32]);
            DerivedKey(bytes)
        };
        let s1 = derive_delegation_seed(&id_a, &salt, "JOY", "ai:claude@joy");
        let s2 = derive_delegation_seed(&id_b, &salt, "JOY", "ai:claude@joy");
        assert_ne!(s1, s2);
    }
}
