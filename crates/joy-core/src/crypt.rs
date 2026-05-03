// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Crypt zone keys and per-member wraps (ADR-038, Crypt.md).
//!
//! Each Crypt zone has one random AES-256-GCM key. Per-member access is
//! granted by wrapping that key under a KEK derived from the member's
//! identity seed. Going through the seed (not the passphrase) means
//! passphrase rotation does not invalidate Crypt access (per ADR-039).
//!
//! This module owns the key-management logic and the project.yaml
//! state transitions. The CLI surface (`joy crypt add/rm/grant/...`)
//! lives in `joy-cli/commands/crypt.rs`.

use joy_crypt::kdf::derive_hkdf_sha256;
use joy_crypt::wrap;
use rand::RngCore;
use zeroize::Zeroizing;

use crate::error::JoyError;

/// Conventional name of the implicit default zone.
pub const DEFAULT_ZONE: &str = "default";

/// 32-byte AES-256-GCM key for a Crypt zone. Lives only in memory;
/// stored in project.yaml as per-member wraps.
pub struct ZoneKey(Zeroizing<[u8; 32]>);

impl std::fmt::Debug for ZoneKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ZoneKey(***)")
    }
}

impl ZoneKey {
    /// Generate a fresh random zone key.
    pub fn generate() -> Self {
        let mut bytes = Zeroizing::new([0u8; 32]);
        rand::thread_rng().fill_bytes(bytes.as_mut());
        Self(bytes)
    }

    /// Reconstruct from raw bytes (e.g. after an unwrap).
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Derive a member's KEK for wrapping zone keys. The KEK is HKDF-SHA256
/// over the member's identity seed, salted with the zone name so that
/// the same seed yields distinct KEKs per zone (and an attacker who
/// recovers one wrap learns nothing about the member's other zones).
fn member_kek(member_seed: &[u8; 32], zone_name: &str) -> [u8; 32] {
    derive_hkdf_sha256(member_seed, zone_name.as_bytes(), b"crypt-member-kek")
}

/// Wrap a zone key for a specific member. Returns the hex-encoded
/// `nonce || ciphertext || tag`.
pub fn wrap_for_member(zone_key: &ZoneKey, zone_name: &str, member_seed: &[u8; 32]) -> String {
    let kek = member_kek(member_seed, zone_name);
    let wrapped = wrap::wrap(&kek, zone_key.as_bytes());
    hex::encode(wrapped)
}

/// Unwrap a zone key for a specific member.
pub fn unwrap_for_member(
    wrap_hex: &str,
    zone_name: &str,
    member_seed: &[u8; 32],
) -> Result<ZoneKey, JoyError> {
    let wrapped = hex::decode(wrap_hex)
        .map_err(|e| JoyError::AuthFailed(format!("invalid crypt wrap: {e}")))?;
    let kek = member_kek(member_seed, zone_name);
    let plain = wrap::unwrap(&kek, &wrapped)
        .map_err(|_| JoyError::AuthFailed(format!("failed to unwrap zone {zone_name}")))?;
    let arr: [u8; 32] = plain.try_into().map_err(|v: Vec<u8>| {
        JoyError::AuthFailed(format!("zone key has wrong length: {}", v.len()))
    })?;
    Ok(ZoneKey::from_bytes(arr))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_seed() -> [u8; 32] {
        [42u8; 32]
    }

    #[test]
    fn wrap_unwrap_roundtrip() {
        let zk = ZoneKey::generate();
        let seed = fixed_seed();
        let wrap_hex = wrap_for_member(&zk, DEFAULT_ZONE, &seed);
        let recovered = unwrap_for_member(&wrap_hex, DEFAULT_ZONE, &seed).unwrap();
        assert_eq!(zk.as_bytes(), recovered.as_bytes());
    }

    #[test]
    fn wrong_seed_rejected() {
        let zk = ZoneKey::generate();
        let wrap_hex = wrap_for_member(&zk, DEFAULT_ZONE, &fixed_seed());
        let other_seed: [u8; 32] = [9u8; 32];
        let err = unwrap_for_member(&wrap_hex, DEFAULT_ZONE, &other_seed).unwrap_err();
        assert!(matches!(err, JoyError::AuthFailed(_)));
    }

    #[test]
    fn wrong_zone_rejected() {
        let zk = ZoneKey::generate();
        let seed = fixed_seed();
        let wrap_hex = wrap_for_member(&zk, "default", &seed);
        // Same seed but different zone name → different KEK → unwrap fails.
        let err = unwrap_for_member(&wrap_hex, "customer-x", &seed).unwrap_err();
        assert!(matches!(err, JoyError::AuthFailed(_)));
    }

    #[test]
    fn distinct_members_get_distinct_wraps() {
        let zk = ZoneKey::generate();
        let a = wrap_for_member(&zk, DEFAULT_ZONE, &[1u8; 32]);
        let b = wrap_for_member(&zk, DEFAULT_ZONE, &[2u8; 32]);
        assert_ne!(a, b);
    }
}
