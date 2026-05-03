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

/// Magic prefix for Crypt-encrypted file blobs (Git filter format).
pub const FILTER_MAGIC: &[u8; 8] = b"JOYCRYPT";
/// On-disk blob format version. Bump on incompatible changes.
pub const FILTER_VERSION: u8 = 1;

/// Encrypt content for a zone in the Git-filter blob format
/// (JOY-014B-09):
///
/// `JOYCRYPT || version (1) || zone-name-len (1) || zone-name ||
///  nonce (12) || ciphertext+tag`.
///
/// The magic header is required so the filter (`joy crypt-filter clean`)
/// can refuse to operate on plaintext that was already encrypted, and
/// the textconv path can short-circuit on already-plaintext history
/// objects.
pub fn encrypt_blob(zone_name: &str, zone_key: &ZoneKey, plaintext: &[u8]) -> Vec<u8> {
    let zone_bytes = zone_name.as_bytes();
    assert!(zone_bytes.len() <= 255, "zone name too long for blob format");
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let aad = aad_for(zone_bytes);
    let ct = joy_crypt::aead::seal(zone_key.as_bytes(), &nonce, &aad, plaintext)
        .expect("AES-256-GCM seal with valid 32-byte key never fails");

    let mut out = Vec::with_capacity(8 + 1 + 1 + zone_bytes.len() + 12 + ct.len());
    out.extend_from_slice(FILTER_MAGIC);
    out.push(FILTER_VERSION);
    out.push(zone_bytes.len() as u8);
    out.extend_from_slice(zone_bytes);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

/// Inverse of `encrypt_blob`. Returns `(zone_name, plaintext)`.
pub fn decrypt_blob(zone_key_lookup: impl Fn(&str) -> Option<ZoneKey>, blob: &[u8]) -> Result<(String, Vec<u8>), JoyError> {
    if blob.len() < 8 + 1 + 1 + 12 + 16 || &blob[..8] != FILTER_MAGIC {
        return Err(JoyError::AuthFailed("not a Crypt blob".into()));
    }
    let version = blob[8];
    if version != FILTER_VERSION {
        return Err(JoyError::AuthFailed(format!(
            "unsupported Crypt blob version: {}",
            version
        )));
    }
    let zone_len = blob[9] as usize;
    let zone_start = 10;
    let zone_end = zone_start + zone_len;
    if blob.len() < zone_end + 12 + 16 {
        return Err(JoyError::AuthFailed("truncated Crypt blob".into()));
    }
    let zone_name = std::str::from_utf8(&blob[zone_start..zone_end])
        .map_err(|_| JoyError::AuthFailed("invalid zone name in Crypt blob".into()))?
        .to_string();
    let nonce_start = zone_end;
    let nonce_end = nonce_start + 12;
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&blob[nonce_start..nonce_end]);
    let ct = &blob[nonce_end..];

    let zone_key = zone_key_lookup(&zone_name)
        .ok_or_else(|| JoyError::AuthFailed(format!("no zone key for '{}'; run `joy auth`", zone_name)))?;
    let aad = aad_for(zone_name.as_bytes());
    let plaintext = joy_crypt::aead::open(zone_key.as_bytes(), &nonce, &aad, ct)
        .map_err(|_| JoyError::AuthFailed(format!("failed to decrypt zone '{}'", zone_name)))?;
    Ok((zone_name, plaintext))
}

/// AAD binds the ciphertext to its zone-name so a wrap from one zone
/// can't be replayed under another.
fn aad_for(zone_bytes: &[u8]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(8 + zone_bytes.len());
    aad.extend_from_slice(b"JOYBLOB:");
    aad.extend_from_slice(zone_bytes);
    aad
}

/// Quick check for whether a byte slice begins with the Crypt blob
/// magic. Used by the filter to short-circuit when working-dir
/// content is already plaintext (e.g. on first add before encryption
/// landed).
pub fn looks_like_blob(bytes: &[u8]) -> bool {
    bytes.len() >= FILTER_MAGIC.len() && &bytes[..FILTER_MAGIC.len()] == FILTER_MAGIC
}

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
    fn blob_roundtrip() {
        let zk = ZoneKey::generate();
        let pt = b"id: JOY-0123\ntitle: secret\n";
        let blob = encrypt_blob("default", &zk, pt);
        assert!(looks_like_blob(&blob));
        let (zone, recovered) =
            decrypt_blob(|name| if name == "default" { Some(ZoneKey::from_bytes(*zk.as_bytes())) } else { None }, &blob)
                .unwrap();
        assert_eq!(zone, "default");
        assert_eq!(recovered, pt);
    }

    #[test]
    fn blob_rejects_wrong_zone_key() {
        let zk = ZoneKey::generate();
        let blob = encrypt_blob("default", &zk, b"x");
        let other = ZoneKey::generate();
        let err = decrypt_blob(|_| Some(ZoneKey::from_bytes(*other.as_bytes())), &blob).unwrap_err();
        assert!(matches!(err, JoyError::AuthFailed(_)));
    }

    #[test]
    fn blob_rejects_tampered_zone_name() {
        let zk = ZoneKey::generate();
        let mut blob = encrypt_blob("default", &zk, b"x");
        // Flip a byte inside the zone name (offset 10).
        blob[10] ^= 1;
        let zk_clone = ZoneKey::from_bytes(*zk.as_bytes());
        let err = decrypt_blob(|_| Some(ZoneKey::from_bytes(*zk_clone.as_bytes())), &blob).unwrap_err();
        assert!(matches!(err, JoyError::AuthFailed(_)));
    }

    #[test]
    fn looks_like_blob_detects_magic() {
        assert!(looks_like_blob(b"JOYCRYPT\x01\x07default..."));
        assert!(!looks_like_blob(b"id: JOY-0123\n"));
        assert!(!looks_like_blob(b""));
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
