// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Wrapped-seed identity helpers (ADR-039).
//!
//! Each member holds a randomly generated 32-byte seed. The Ed25519
//! identity keypair derives from this seed. The seed is stored in
//! `project.yaml` as two AES-256-GCM ciphertexts:
//!
//! - `seed_wrap_passphrase`: encrypted under a KEK derived from
//!   `passphrase + kdf_nonce` via Argon2id.
//! - `seed_wrap_recovery`: encrypted under a KEK derived from a
//!   recovery key via Argon2id (same `kdf_nonce`).
//!
//! Either wrap unlocks the same seed; the keypair stays stable across
//! passphrase rotation. The recovery key itself is displayed once at
//! `joy auth init` and stored externally by the user.

use joy_crypt::kdf::{derive_argon2id, DerivedKey, Salt};
use joy_crypt::wrap;
use rand::RngCore;
use zeroize::Zeroizing;

use crate::error::JoyError;

/// 32-byte recovery key. Displayed once at `joy auth init`. Stored
/// externally by the user; never persisted by Joy.
pub struct RecoveryKey(Zeroizing<[u8; 32]>);

impl std::fmt::Debug for RecoveryKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecoveryKey(***)")
    }
}

impl RecoveryKey {
    /// Generate a fresh random recovery key.
    pub fn generate() -> Self {
        let mut bytes = Zeroizing::new([0u8; 32]);
        rand::thread_rng().fill_bytes(bytes.as_mut());
        Self(bytes)
    }

    /// Encode for one-time display (hex with `joy_r_` prefix).
    pub fn to_display_string(&self) -> String {
        format!("joy_r_{}", hex::encode(self.0.as_ref()))
    }

    /// Parse a user-supplied recovery key. Accepts the `joy_r_` prefix
    /// or bare hex.
    pub fn from_user_input(s: &str) -> Result<Self, JoyError> {
        let trimmed = s.trim().trim_start_matches("joy_r_");
        let bytes = hex::decode(trimmed)
            .map_err(|e| JoyError::AuthFailed(format!("invalid recovery key: {e}")))?;
        let arr: [u8; 32] = bytes.try_into().map_err(|v: Vec<u8>| {
            JoyError::AuthFailed(format!(
                "recovery key must be 32 bytes ({} hex chars), got {} bytes",
                64,
                v.len()
            ))
        })?;
        Ok(Self(Zeroizing::new(arr)))
    }

    /// Bytes form for KDF input.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// 32-byte identity seed. The Ed25519 keypair derives from this value.
pub struct Seed(Zeroizing<[u8; 32]>);

impl std::fmt::Debug for Seed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Seed(***)")
    }
}

impl Seed {
    /// Generate a fresh random seed.
    pub fn generate() -> Self {
        let mut bytes = Zeroizing::new([0u8; 32]);
        rand::thread_rng().fill_bytes(bytes.as_mut());
        Self(bytes)
    }

    /// Wrap a known 32-byte seed (used by lazy migration where the seed
    /// is the legacy Argon2id-derived key).
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// Construct a seed from an Argon2id `DerivedKey` (used by lazy
    /// migration: legacy KEK_passphrase becomes the seed).
    pub fn from_derived_key(key: &DerivedKey) -> Self {
        Self::from_bytes(*key.as_bytes())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Derive the passphrase KEK and wrap a seed under it. Returns the
/// hex-encoded `nonce || ciphertext || tag`.
pub fn wrap_seed_with_passphrase(
    seed: &Seed,
    passphrase: &str,
    kdf_nonce: &Salt,
) -> Result<String, JoyError> {
    let kek = derive_argon2id(passphrase, kdf_nonce)?;
    let wrapped = wrap::wrap(kek.as_bytes(), seed.as_bytes());
    Ok(hex::encode(wrapped))
}

/// Derive the recovery KEK and wrap a seed under it. Returns hex.
pub fn wrap_seed_with_recovery(
    seed: &Seed,
    recovery_key: &RecoveryKey,
    kdf_nonce: &Salt,
) -> Result<String, JoyError> {
    let pass = hex::encode(recovery_key.as_bytes());
    let kek = derive_argon2id(&pass, kdf_nonce)?;
    let wrapped = wrap::wrap(kek.as_bytes(), seed.as_bytes());
    Ok(hex::encode(wrapped))
}

/// Unwrap a seed via the passphrase KEK. Returns the 32-byte seed on
/// success; AES-GCM auth failure indicates wrong passphrase.
pub fn unwrap_seed_with_passphrase(
    wrap_hex: &str,
    passphrase: &str,
    kdf_nonce: &Salt,
) -> Result<Seed, JoyError> {
    let wrapped = hex::decode(wrap_hex)
        .map_err(|e| JoyError::AuthFailed(format!("invalid seed_wrap_passphrase: {e}")))?;
    let kek = derive_argon2id(passphrase, kdf_nonce)?;
    let plain = wrap::unwrap(kek.as_bytes(), &wrapped)
        .map_err(|_| JoyError::AuthFailed("incorrect passphrase".into()))?;
    let arr: [u8; 32] = plain.try_into().map_err(|v: Vec<u8>| {
        JoyError::AuthFailed(format!("seed has wrong length: {}", v.len()))
    })?;
    Ok(Seed::from_bytes(arr))
}

/// Lazy-migration wrap (ADR-039 §"Migration is non-disruptive"): when
/// the legacy Argon2id-derived KEK already equals the seed (because the
/// pre-wrapped-seed model used the KEK directly as the Ed25519 seed),
/// wrap the seed under itself. Subsequent passphrase changes decouple
/// the KEK from the seed naturally.
pub fn wrap_seed_for_migration(seed: &Seed) -> String {
    let wrapped = wrap::wrap(seed.as_bytes(), seed.as_bytes());
    hex::encode(wrapped)
}

/// Unwrap a seed via the recovery KEK. AES-GCM auth failure indicates
/// wrong recovery key.
pub fn unwrap_seed_with_recovery(
    wrap_hex: &str,
    recovery_key: &RecoveryKey,
    kdf_nonce: &Salt,
) -> Result<Seed, JoyError> {
    let wrapped = hex::decode(wrap_hex)
        .map_err(|e| JoyError::AuthFailed(format!("invalid seed_wrap_recovery: {e}")))?;
    let pass = hex::encode(recovery_key.as_bytes());
    let kek = derive_argon2id(&pass, kdf_nonce)?;
    let plain = wrap::unwrap(kek.as_bytes(), &wrapped)
        .map_err(|_| JoyError::AuthFailed("incorrect recovery key".into()))?;
    let arr: [u8; 32] = plain.try_into().map_err(|v: Vec<u8>| {
        JoyError::AuthFailed(format!("seed has wrong length: {}", v.len()))
    })?;
    Ok(Seed::from_bytes(arr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use joy_crypt::identity::Keypair;
    use joy_crypt::kdf::generate_salt;

    #[test]
    fn passphrase_wrap_roundtrip() {
        let seed = Seed::generate();
        let salt = generate_salt();
        let wrap_hex =
            wrap_seed_with_passphrase(&seed, "correct horse battery staple foo bar", &salt)
                .unwrap();
        let recovered =
            unwrap_seed_with_passphrase(&wrap_hex, "correct horse battery staple foo bar", &salt)
                .unwrap();
        assert_eq!(seed.as_bytes(), recovered.as_bytes());
    }

    #[test]
    fn passphrase_wrong_passphrase_rejected() {
        let seed = Seed::generate();
        let salt = generate_salt();
        let wrap_hex =
            wrap_seed_with_passphrase(&seed, "right pass words six total", &salt).unwrap();
        let err = unwrap_seed_with_passphrase(&wrap_hex, "wrong pass words six total here", &salt)
            .unwrap_err();
        assert!(matches!(err, JoyError::AuthFailed(_)));
    }

    #[test]
    fn recovery_wrap_roundtrip() {
        let seed = Seed::generate();
        let salt = generate_salt();
        let recovery = RecoveryKey::generate();
        let wrap_hex = wrap_seed_with_recovery(&seed, &recovery, &salt).unwrap();
        let recovered = unwrap_seed_with_recovery(&wrap_hex, &recovery, &salt).unwrap();
        assert_eq!(seed.as_bytes(), recovered.as_bytes());
    }

    #[test]
    fn recovery_wrong_key_rejected() {
        let seed = Seed::generate();
        let salt = generate_salt();
        let recovery = RecoveryKey::generate();
        let wrap_hex = wrap_seed_with_recovery(&seed, &recovery, &salt).unwrap();
        let other = RecoveryKey::generate();
        let err = unwrap_seed_with_recovery(&wrap_hex, &other, &salt).unwrap_err();
        assert!(matches!(err, JoyError::AuthFailed(_)));
    }

    #[test]
    fn passphrase_change_preserves_keypair() {
        // Wrap a seed, then re-wrap under a new passphrase; the keypair
        // derived from the seed must be identical.
        let seed = Seed::generate();
        let kp_before = Keypair::from_seed(seed.as_bytes());

        let salt = generate_salt();
        let old_wrap =
            wrap_seed_with_passphrase(&seed, "alpha bravo charlie delta echo foxtrot", &salt)
                .unwrap();
        let recovered =
            unwrap_seed_with_passphrase(&old_wrap, "alpha bravo charlie delta echo foxtrot", &salt)
                .unwrap();
        let new_wrap =
            wrap_seed_with_passphrase(&recovered, "yankee zulu papa quebec sierra tango", &salt)
                .unwrap();
        let after =
            unwrap_seed_with_passphrase(&new_wrap, "yankee zulu papa quebec sierra tango", &salt)
                .unwrap();
        let kp_after = Keypair::from_seed(after.as_bytes());

        assert_eq!(kp_before.public_key(), kp_after.public_key());
    }

    #[test]
    fn recovery_key_display_roundtrip() {
        let r = RecoveryKey::generate();
        let s = r.to_display_string();
        assert!(s.starts_with("joy_r_"));
        let parsed = RecoveryKey::from_user_input(&s).unwrap();
        assert_eq!(r.as_bytes(), parsed.as_bytes());
    }

    #[test]
    fn recovery_key_accepts_bare_hex() {
        let r = RecoveryKey::generate();
        let bare = hex::encode(r.as_bytes());
        let parsed = RecoveryKey::from_user_input(&bare).unwrap();
        assert_eq!(r.as_bytes(), parsed.as_bytes());
    }

    #[test]
    fn recovery_key_rejects_bad_input() {
        assert!(RecoveryKey::from_user_input("zzz").is_err());
        assert!(RecoveryKey::from_user_input("joy_r_00").is_err());
    }
}
