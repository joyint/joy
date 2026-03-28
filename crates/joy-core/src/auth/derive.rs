// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Argon2id key derivation from passphrase and project-bound salt.
//!
//! Produces 32 bytes of key material suitable for Ed25519 seed generation.
//! Parameters match Bitwarden defaults: 64 MiB memory, 3 iterations, 4 lanes.

use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;
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
/// Parameters: m_cost=65536 (64 MiB), t_cost=3, p_cost=4, output=32 bytes.
pub fn derive_key(passphrase: &str, salt: &Salt) -> Result<DerivedKey, JoyError> {
    let params = Params::new(65536, 3, 4, Some(32))
        .map_err(|e| JoyError::AuthFailed(format!("argon2 params: {e}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let mut output = Zeroizing::new([0u8; 32]);
    argon2
        .hash_password_into(passphrase.as_bytes(), salt.as_bytes(), output.as_mut())
        .map_err(|e| JoyError::AuthFailed(format!("key derivation failed: {e}")))?;

    Ok(DerivedKey(output))
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
}
