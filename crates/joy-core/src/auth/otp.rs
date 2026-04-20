// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! One-time password generation and verification for member onboarding.
//!
//! The admin runs `joy project member add <email>`, which emits an OTP
//! that is shared out-of-band with the new member. The new member runs
//! `joy auth --otp <code>` to redeem it, which unlocks setting their own
//! passphrase.
//!
//! Storage format in `project.yaml` member.otp_hash field:
//! `"<salt_hex>:<hash_hex>"` where both halves are 32 hex-encoded bytes.
//! Argon2id with the derive-module's parameters is reused so debug and
//! `fast-kdf` builds stay cheap.

use argon2::{Algorithm, Argon2, Params, Version};
use rand::distributions::{Alphanumeric, DistString};
use rand::RngCore;

use super::derive;
use crate::error::JoyError;

/// Generate a fresh OTP formatted as `XXXX-XXXX-XXXX` using uppercase
/// alphanumeric characters (no ambiguous ones removed, kept simple).
pub fn generate_otp() -> String {
    let raw = Alphanumeric
        .sample_string(&mut rand::thread_rng(), 12)
        .to_uppercase();
    format!("{}-{}-{}", &raw[0..4], &raw[4..8], &raw[8..12])
}

/// Hash an OTP for storage in `project.yaml member.otp_hash`.
/// Returns `"<salt_hex>:<hash_hex>"`.
pub fn hash_otp(otp: &str) -> Result<String, JoyError> {
    let mut salt = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut salt);
    let hash = argon2id_raw(otp.as_bytes(), &salt)?;
    Ok(format!("{}:{}", hex::encode(salt), hex::encode(hash)))
}

/// Verify a plaintext OTP against a stored `<salt_hex>:<hash_hex>` string.
pub fn verify_otp(otp: &str, stored: &str) -> Result<bool, JoyError> {
    let (salt_hex, hash_hex) = stored.split_once(':').ok_or_else(|| {
        JoyError::AuthFailed("otp_hash has wrong format (expected salt:hash)".into())
    })?;
    let salt = hex::decode(salt_hex)
        .map_err(|e| JoyError::AuthFailed(format!("invalid otp salt: {e}")))?;
    let expected = hex::decode(hash_hex)
        .map_err(|e| JoyError::AuthFailed(format!("invalid otp hash: {e}")))?;
    let actual = argon2id_raw(otp.as_bytes(), &salt)?;
    // constant-time comparison
    Ok(constant_time_eq(&actual, &expected))
}

fn argon2id_raw(material: &[u8], salt: &[u8]) -> Result<[u8; 32], JoyError> {
    // Mirror derive::derive_key params so debug/fast-kdf builds are cheap.
    let _ = &derive::generate_salt; // silence unused-import warnings in future refactors
    #[cfg(any(feature = "fast-kdf", debug_assertions))]
    let params = Params::new(256, 1, 1, Some(32))
        .map_err(|e| JoyError::AuthFailed(format!("argon2 params: {e}")))?;
    #[cfg(not(any(feature = "fast-kdf", debug_assertions)))]
    let params = Params::new(65536, 3, 4, Some(32))
        .map_err(|e| JoyError::AuthFailed(format!("argon2 params: {e}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon2
        .hash_password_into(material, salt, &mut out)
        .map_err(|e| JoyError::AuthFailed(format!("otp hashing failed: {e}")))?;
    Ok(out)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut d = 0u8;
    for i in 0..a.len() {
        d |= a[i] ^ b[i];
    }
    d == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn otp_format() {
        let otp = generate_otp();
        assert_eq!(otp.len(), 14);
        assert_eq!(otp.chars().filter(|c| *c == '-').count(), 2);
        assert!(otp.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }

    #[test]
    fn hash_and_verify_roundtrip() {
        let otp = generate_otp();
        let stored = hash_otp(&otp).unwrap();
        assert!(verify_otp(&otp, &stored).unwrap());
        assert!(!verify_otp("WRONG-CODE-1234", &stored).unwrap());
    }

    #[test]
    fn malformed_stored_hash_errors() {
        let otp = "ABCD-EFGH-IJKL";
        let err = verify_otp(otp, "no-colon-here").unwrap_err();
        assert!(matches!(err, JoyError::AuthFailed(_)));
    }
}
