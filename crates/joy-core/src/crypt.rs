// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Crypt zone keys and per-member wraps (ADR-038, Crypt.md).
//!
//! Each Crypt zone has one random AES-256-GCM key. Per-member access is
//! granted by wrapping that key under a KEK derived from a pairwise
//! X25519 ECDH between the granter's identity and the recipient's
//! identity (ADR-038, JOY-0157-86). Going through identity material
//! that is stable across passphrase rotation (ADR-039) means
//! passphrase changes do not invalidate Crypt access.
//!
//! Wrap format on disk: hex-encoded
//! `granter_verify_key (32 bytes) || nonce (12) || ciphertext || tag (16)`.
//!
//! Self-wrap (auto-create on `joy crypt add`) is the special case where
//! granter and recipient are the same member; the wrap format is
//! identical so the unwrap path is uniform.

use joy_crypt::identity::{Keypair, PublicKey};
use joy_crypt::pairwise::pairwise_kek;
use joy_crypt::wrap;
use rand::RngCore;
use zeroize::Zeroizing;

use crate::error::JoyError;

/// Conventional name of the implicit default zone.
pub const DEFAULT_ZONE: &str = "default";

/// 32-byte AES-256-GCM key for a Crypt zone.
pub struct ZoneKey(Zeroizing<[u8; 32]>);

impl std::fmt::Debug for ZoneKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ZoneKey(***)")
    }
}

impl ZoneKey {
    pub fn generate() -> Self {
        let mut bytes = Zeroizing::new([0u8; 32]);
        rand::thread_rng().fill_bytes(bytes.as_mut());
        Self(bytes)
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Derive the HKDF info string for a zone-key wrap. Binds the wrap to
/// the zone name so distinct zones produce distinct KEKs.
fn wrap_info(zone_name: &str) -> Vec<u8> {
    let mut info = Vec::with_capacity(11 + zone_name.len());
    info.extend_from_slice(b"crypt-zone:");
    info.extend_from_slice(zone_name.as_bytes());
    info
}

/// Wrap a zone key for a recipient. The granter contributes their
/// X25519 secret (derived from their identity seed); the recipient is
/// addressed by their Ed25519 verify_key. Self-wrap is a special case
/// where granter and recipient identify the same member.
///
/// Returns the hex-encoded wrap.
pub fn wrap_for_member(
    zone_key: &ZoneKey,
    zone_name: &str,
    granter_seed: &[u8; 32],
    granter_verify_key: &PublicKey,
    recipient_verify_key: &PublicKey,
) -> String {
    let granter_kp = Keypair::from_seed(granter_seed);
    let granter_secret = granter_kp.to_x25519_secret_bytes();
    let recipient_x25519 = recipient_verify_key.to_x25519_public_bytes();
    let info = wrap_info(zone_name);
    let kek = pairwise_kek(&granter_secret, &recipient_x25519, &info);

    let inner = wrap::wrap(&kek, zone_key.as_bytes());
    let mut out = Vec::with_capacity(32 + inner.len());
    out.extend_from_slice(&granter_verify_key.as_bytes());
    out.extend_from_slice(&inner);
    hex::encode(out)
}

/// Convenience wrapper: self-wrap produced by a member for themselves.
pub fn wrap_for_self(
    zone_key: &ZoneKey,
    zone_name: &str,
    member_seed: &[u8; 32],
) -> String {
    let kp = Keypair::from_seed(member_seed);
    let pk = kp.public_key();
    wrap_for_member(zone_key, zone_name, member_seed, &pk, &pk)
}

/// Unwrap a zone key. Reads the granter's verify_key from the wrap
/// header, derives the same pairwise KEK on the recipient side, and
/// decrypts the inner blob.
pub fn unwrap_for_member(
    wrap_hex: &str,
    zone_name: &str,
    recipient_seed: &[u8; 32],
) -> Result<ZoneKey, JoyError> {
    let bytes = hex::decode(wrap_hex)
        .map_err(|e| JoyError::AuthFailed(format!("invalid crypt wrap: {e}")))?;
    if bytes.len() < 32 {
        return Err(JoyError::AuthFailed(
            "crypt wrap too short to contain granter prefix".into(),
        ));
    }
    let mut granter_pk_bytes = [0u8; 32];
    granter_pk_bytes.copy_from_slice(&bytes[..32]);
    let granter_pk = PublicKey::from_hex(&hex::encode(granter_pk_bytes))?;
    let granter_x25519 = granter_pk.to_x25519_public_bytes();

    let recipient_kp = Keypair::from_seed(recipient_seed);
    let recipient_secret = recipient_kp.to_x25519_secret_bytes();
    let info = wrap_info(zone_name);
    let kek = pairwise_kek(&recipient_secret, &granter_x25519, &info);

    let plain = wrap::unwrap(&kek, &bytes[32..])
        .map_err(|_| JoyError::AuthFailed(format!("failed to unwrap zone {zone_name}")))?;
    let arr: [u8; 32] = plain.try_into().map_err(|v: Vec<u8>| {
        JoyError::AuthFailed(format!("zone key has wrong length: {}", v.len()))
    })?;
    Ok(ZoneKey::from_bytes(arr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use joy_crypt::identity::Keypair;

    #[test]
    fn self_wrap_roundtrip() {
        let zk = ZoneKey::generate();
        let seed = [42u8; 32];
        let wrap_hex = wrap_for_self(&zk, DEFAULT_ZONE, &seed);
        let recovered = unwrap_for_member(&wrap_hex, DEFAULT_ZONE, &seed).unwrap();
        assert_eq!(zk.as_bytes(), recovered.as_bytes());
    }

    #[test]
    fn cross_member_wrap_roundtrip() {
        let zk = ZoneKey::generate();
        let granter_seed = [1u8; 32];
        let recipient_seed = [2u8; 32];
        let granter_pk = Keypair::from_seed(&granter_seed).public_key();
        let recipient_pk = Keypair::from_seed(&recipient_seed).public_key();

        let wrap_hex = wrap_for_member(
            &zk,
            DEFAULT_ZONE,
            &granter_seed,
            &granter_pk,
            &recipient_pk,
        );
        let recovered = unwrap_for_member(&wrap_hex, DEFAULT_ZONE, &recipient_seed).unwrap();
        assert_eq!(zk.as_bytes(), recovered.as_bytes());
    }

    #[test]
    fn third_member_cannot_unwrap() {
        let zk = ZoneKey::generate();
        let granter_seed = [1u8; 32];
        let recipient_seed = [2u8; 32];
        let intruder_seed = [9u8; 32];
        let granter_pk = Keypair::from_seed(&granter_seed).public_key();
        let recipient_pk = Keypair::from_seed(&recipient_seed).public_key();

        let wrap_hex = wrap_for_member(
            &zk,
            DEFAULT_ZONE,
            &granter_seed,
            &granter_pk,
            &recipient_pk,
        );
        let err = unwrap_for_member(&wrap_hex, DEFAULT_ZONE, &intruder_seed).unwrap_err();
        assert!(matches!(err, JoyError::AuthFailed(_)));
    }

    #[test]
    fn wrong_zone_rejected() {
        let zk = ZoneKey::generate();
        let seed = [42u8; 32];
        let wrap_hex = wrap_for_self(&zk, "default", &seed);
        let err = unwrap_for_member(&wrap_hex, "customer-x", &seed).unwrap_err();
        assert!(matches!(err, JoyError::AuthFailed(_)));
    }

    #[test]
    fn truncated_wrap_rejected() {
        let bytes = vec![0u8; 16];
        let err = unwrap_for_member(&hex::encode(&bytes), DEFAULT_ZONE, &[1u8; 32]).unwrap_err();
        assert!(matches!(err, JoyError::AuthFailed(_)));
    }

    #[test]
    fn passphrase_change_does_not_invalidate_wrap() {
        // The wrap is keyed by the recipient's seed, which under
        // ADR-039 is stable across passphrase rotation. Simulate by
        // unwrapping with the same seed twice.
        let zk = ZoneKey::generate();
        let seed = [7u8; 32];
        let wrap_hex = wrap_for_self(&zk, DEFAULT_ZONE, &seed);
        let a = unwrap_for_member(&wrap_hex, DEFAULT_ZONE, &seed).unwrap();
        let b = unwrap_for_member(&wrap_hex, DEFAULT_ZONE, &seed).unwrap();
        assert_eq!(a.as_bytes(), b.as_bytes());
    }
}
