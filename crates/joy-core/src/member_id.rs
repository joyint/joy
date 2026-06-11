// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Anonymous-mode member primitives (ADR-042).
//!
//! Two pure, deterministic functions that the platform (joyint.com) must
//! reproduce bit-for-bit, so their parameters are pinned here and treated as a
//! cross-implementation contract:
//!
//! * [`opaque_member_id`] derives the at-rest member id (`m-<short>`) from the
//!   member's Ed25519 verify key. It carries no PII and is recomputable by
//!   anyone holding the verify key.
//! * [`email_match`] derives the non-reversible e-mail verifier stored in
//!   `project.yaml` under `email_match`. The platform computes the same value
//!   over a verified account e-mail to decide project membership, without
//!   decrypting anything.
//!
//! Crypto note: ADR-042's prose says "HMAC-SHA256". The implementation uses
//! HKDF-SHA256 (whose extract step *is* HMAC-SHA256) because joy-crypt already
//! exposes it (ADR-039 keeps all primitives in joy-crypt) and it adds domain
//! separation via a fixed `info`. The verifier property (fast, deterministic,
//! non-reversible, equality-checkable) is unchanged.

use sha2::{Digest, Sha256};

/// Fixed HKDF `info` for the e-mail verifier. Domain-separates this use of
/// HKDF-SHA256 from delegation-seed derivation (`b"joy-delegation:..."`).
/// MUST NOT change: the platform keys its comparison on the identical value.
const EMAIL_MATCH_INFO: &[u8] = b"joy-email-match";

/// Length (in base32 chars) of the short part of an opaque member id. 10 chars
/// of base32 = 50 bits, far beyond collision range for members-per-project.
/// Pinned: the platform derives the same id.
const MEMBER_ID_SHORT_LEN: usize = 10;

/// RFC 4648 base32 alphabet, lowercase, no padding. Pinned for cross-impl
/// reproducibility.
const BASE32_ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// Normalize an e-mail for verifier computation: trim surrounding whitespace,
/// lowercase. Nothing else (no Gmail-dot or plus-address handling), per ADR-042.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// Encode bytes as lowercase, unpadded RFC 4648 base32.
fn base32_lower_nopad(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(5) * 8);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in data {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let idx = ((buffer >> bits) & 0x1f) as usize;
            out.push(BASE32_ALPHABET[idx] as char);
        }
    }
    if bits > 0 {
        let idx = ((buffer << (5 - bits)) & 0x1f) as usize;
        out.push(BASE32_ALPHABET[idx] as char);
    }
    out
}

/// Derive the opaque, stable member id `m-<short>` from the member's Ed25519
/// verify key (hex). The short part is the first [`MEMBER_ID_SHORT_LEN`] base32
/// chars of `SHA-256(verify_key_bytes)`. Deterministic and PII-free.
///
/// Returns an error if `verify_key_hex` is not valid hex.
pub fn opaque_member_id(verify_key_hex: &str) -> Result<String, hex::FromHexError> {
    let vk = hex::decode(verify_key_hex.trim())?;
    let digest = Sha256::digest(&vk);
    let b32 = base32_lower_nopad(&digest);
    Ok(format!("m-{}", &b32[..MEMBER_ID_SHORT_LEN]))
}

/// Whether `s` has the shape of an opaque member id: `m-` followed by exactly
/// [`MEMBER_ID_SHORT_LEN`] base32 (lowercase, no padding) characters. Lets
/// anonymous-mode ids be accepted wherever an e-mail or `ai:` id is otherwise
/// expected (e.g. self-assign resolves to the opaque id).
pub fn is_opaque_member_id(s: &str) -> bool {
    match s.strip_prefix("m-") {
        Some(rest) => {
            rest.len() == MEMBER_ID_SHORT_LEN && rest.bytes().all(|b| BASE32_ALPHABET.contains(&b))
        }
        None => false,
    }
}

/// Compute the non-reversible `email_match` verifier for `email`, keyed by the
/// member's project-stable `kdf_nonce` (hex). Hex-encoded 32-byte output.
///
/// `email_match = hex( HKDF-SHA256(ikm = normalize(email), salt = kdf_nonce,
/// info = "joy-email-match", len = 32) )`.
///
/// Returns an error if `kdf_nonce_hex` is not valid hex.
pub fn email_match(email: &str, kdf_nonce_hex: &str) -> Result<String, hex::FromHexError> {
    let nonce = hex::decode(kdf_nonce_hex.trim())?;
    let normalized = normalize_email(email);
    let out = joy_crypt::kdf::derive_hkdf_sha256(normalized.as_bytes(), &nonce, EMAIL_MATCH_INFO);
    Ok(hex::encode(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    // A valid 32-byte Ed25519 verify key (hex), from the dogfood project.yaml.
    const VK: &str = "05ae823757c1d3d0db5f2f66c7f55642b2258f15776a6b2f029142bfa9b5f73d";
    const NONCE: &str = "8c1f00000000000000000000000000000000000000000000000000000000e4ab";

    #[test]
    fn opaque_member_id_shape_and_determinism() {
        let a = opaque_member_id(VK).unwrap();
        let b = opaque_member_id(VK).unwrap();
        assert_eq!(a, b);
        assert!(a.starts_with("m-"));
        assert_eq!(a.len(), 2 + MEMBER_ID_SHORT_LEN);
        assert!(a[2..].bytes().all(|c| BASE32_ALPHABET.contains(&c)));
    }

    #[test]
    fn opaque_member_id_differs_by_key() {
        let other = "1111111111111111111111111111111111111111111111111111111111111111";
        assert_ne!(
            opaque_member_id(VK).unwrap(),
            opaque_member_id(other).unwrap()
        );
    }

    #[test]
    fn opaque_member_id_rejects_bad_hex() {
        assert!(opaque_member_id("nothex").is_err());
    }

    #[test]
    fn email_match_is_deterministic_and_32_bytes() {
        let a = email_match("horst@joydev.com", NONCE).unwrap();
        let b = email_match("horst@joydev.com", NONCE).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64); // 32 bytes hex
    }

    #[test]
    fn email_match_normalizes_case_and_whitespace() {
        let plain = email_match("horst@joydev.com", NONCE).unwrap();
        let messy = email_match("  Horst@JoyDev.com  ", NONCE).unwrap();
        assert_eq!(plain, messy);
    }

    #[test]
    fn email_match_differs_by_email() {
        assert_ne!(
            email_match("a@example.com", NONCE).unwrap(),
            email_match("b@example.com", NONCE).unwrap()
        );
    }

    #[test]
    fn email_match_differs_by_nonce() {
        let other = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        assert_ne!(
            email_match("a@example.com", NONCE).unwrap(),
            email_match("a@example.com", other).unwrap()
        );
    }

    #[test]
    fn base32_known_vector() {
        // RFC 4648 base32 of "foobar" is MZXW6YTBOI; lowercase, unpadded here.
        assert_eq!(base32_lower_nopad(b"foobar"), "mzxw6ytboi");
    }
}
