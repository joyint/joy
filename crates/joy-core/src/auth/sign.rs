// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Ed25519 identity keypair for signing and verification.
//!
//! The keypair is derived deterministically from a `DerivedKey` (Argon2id output).
//! The private key exists only transiently in memory and is zeroed on drop.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use super::derive::DerivedKey;
use crate::error::JoyError;

/// Ed25519 signing keypair. Private key is zeroed on drop.
pub struct IdentityKeypair {
    signing_key: SigningKey,
}

/// Ed25519 public key. Stored in project.yaml per member as hex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey(VerifyingKey);

impl IdentityKeypair {
    /// Create a keypair from derived key material (32-byte Ed25519 seed).
    pub fn from_derived_key(key: &DerivedKey) -> Self {
        let signing_key = SigningKey::from_bytes(key.as_bytes());
        Self { signing_key }
    }

    /// Create a keypair from a raw Ed25519 signing key.
    pub fn from_signing_key(key: SigningKey) -> Self {
        Self { signing_key: key }
    }

    /// Create a keypair from a 32-byte seed (e.g. for token-derived sessions).
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(seed);
        Self { signing_key }
    }

    /// Generate a random keypair (for one-time token keys).
    pub fn from_random() -> Self {
        use rand::rngs::OsRng;
        let signing_key = SigningKey::generate(&mut OsRng);
        Self { signing_key }
    }

    /// Derive a deterministic keypair from arbitrary data (e.g. token + project ID).
    /// Uses SHA-256 to produce a 32-byte seed.
    pub fn from_token_seed(token: &str, project_id: &str) -> Self {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        hasher.update(project_id.as_bytes());
        let hash = hasher.finalize();
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&hash);
        Self::from_seed(&seed)
    }

    /// Get the public key for this keypair.
    pub fn public_key(&self) -> PublicKey {
        PublicKey(self.signing_key.verifying_key())
    }

    /// Sign a message with this keypair.
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        let sig: Signature = self.signing_key.sign(message);
        sig.to_bytes().to_vec()
    }

    /// Extract the 32-byte seed for at-rest persistence (e.g. delegation key files).
    /// The returned bytes are the private key material; the caller is responsible
    /// for protecting them (file permissions 0600, etc.).
    pub fn to_seed_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }
}

impl PublicKey {
    /// Verify a signature against this public key.
    pub fn verify(&self, message: &[u8], signature: &[u8]) -> Result<(), JoyError> {
        let sig = Signature::from_slice(signature)
            .map_err(|e| JoyError::AuthFailed(format!("invalid signature: {e}")))?;
        self.0
            .verify(message, &sig)
            .map_err(|_| JoyError::AuthFailed("signature verification failed".into()))
    }

    /// Encode as hex string for storage in project.yaml.
    pub fn to_hex(&self) -> String {
        hex::encode(self.0.as_bytes())
    }

    /// Decode from hex string.
    pub fn from_hex(s: &str) -> Result<Self, JoyError> {
        let bytes =
            hex::decode(s).map_err(|e| JoyError::AuthFailed(format!("invalid public key: {e}")))?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| JoyError::AuthFailed("public key must be 32 bytes".into()))?;
        let key = VerifyingKey::from_bytes(&arr)
            .map_err(|e| JoyError::AuthFailed(format!("invalid Ed25519 key: {e}")))?;
        Ok(Self(key))
    }
}

impl Drop for IdentityKeypair {
    fn drop(&mut self) {
        // SigningKey implements Zeroize via ed25519-dalek
        // The drop is handled automatically, but we implement Drop
        // to document the security intent.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::derive;

    const TEST_PASSPHRASE: &str = "correct horse battery staple extra words";

    fn test_keypair() -> (IdentityKeypair, DerivedKey) {
        let salt = derive::Salt::from_hex(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        let key = derive::derive_key(TEST_PASSPHRASE, &salt).unwrap();
        let keypair = IdentityKeypair::from_derived_key(&key);
        (keypair, key)
    }

    #[test]
    fn keypair_deterministic() {
        let salt = derive::Salt::from_hex(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        let k1 = derive::derive_key(TEST_PASSPHRASE, &salt).unwrap();
        let k2 = derive::derive_key(TEST_PASSPHRASE, &salt).unwrap();
        let kp1 = IdentityKeypair::from_derived_key(&k1);
        let kp2 = IdentityKeypair::from_derived_key(&k2);
        assert_eq!(kp1.public_key(), kp2.public_key());
    }

    #[test]
    fn sign_verify_roundtrip() {
        let (keypair, _) = test_keypair();
        let message = b"hello world";
        let signature = keypair.sign(message);
        assert!(keypair.public_key().verify(message, &signature).is_ok());
    }

    #[test]
    fn verify_wrong_message() {
        let (keypair, _) = test_keypair();
        let signature = keypair.sign(b"original");
        assert!(keypair
            .public_key()
            .verify(b"tampered", &signature)
            .is_err());
    }

    #[test]
    fn verify_wrong_key() {
        let (keypair, _) = test_keypair();
        let signature = keypair.sign(b"hello");

        // Different passphrase = different key
        let salt = derive::generate_salt();
        let other_key =
            derive::derive_key("alpha bravo charlie delta echo foxtrot", &salt).unwrap();
        let other_kp = IdentityKeypair::from_derived_key(&other_key);
        assert!(other_kp.public_key().verify(b"hello", &signature).is_err());
    }

    #[test]
    fn public_key_hex_roundtrip() {
        let (keypair, _) = test_keypair();
        let pk = keypair.public_key();
        let hex = pk.to_hex();
        let parsed = PublicKey::from_hex(&hex).unwrap();
        assert_eq!(pk, parsed);
    }
}
