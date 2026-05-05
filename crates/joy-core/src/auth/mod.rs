// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Cryptographic identity for Joy's Trust Model.
//!
//! Auth provides passphrase-derived Ed25519 identity keys using Argon2id
//! for key derivation. This is the Trustship pillar of AI Governance:
//! it answers "who is this?" with cryptographic proof rather than
//! self-declaration.
//!
//! Key hierarchy:
//! ```text
//! Passphrase + Salt --[Argon2id]--> DerivedKey --[Ed25519]--> Keypair
//! ```
//!
//! Cryptographic primitives (KDF, AEAD, Ed25519, key wrapping) live in
//! the `joy-crypt` crate (ADR-039 §"Crate boundary and dependency
//! direction"). This module owns the identity application layer:
//! sessions, tokens, OTPs, attestations, and the project.yaml schema.

pub mod attestation;
pub mod delegation;
pub mod otp;
pub mod seed;
pub mod session;
pub mod token;

// Re-export joy-crypt primitives under joy-domain names. Callers within
// joy-core/auth and joy-cli use these names; the underlying
// implementation lives in joy-crypt (ADR-039).
pub use joy_crypt::identity::{Keypair as IdentityKeypair, PublicKey};
pub use joy_crypt::kdf::{derive_argon2id as derive_key, generate_salt, DerivedKey, Salt};

use crate::error::JoyError;

/// Validate that a passphrase has at least 6 whitespace-separated words.
///
/// Joy uses the Diceware convention: short list of dictionary words is
/// easier to memorise than random characters and reaches comparable
/// entropy at 6+ words.
pub fn validate_passphrase(passphrase: &str) -> Result<(), JoyError> {
    let word_count = passphrase.split_whitespace().count();
    if word_count < 6 {
        return Err(JoyError::PassphraseTooShort);
    }
    Ok(())
}

/// Result of unlocking a member's identity from a passphrase.
pub struct UnlockedIdentity {
    pub keypair: IdentityKeypair,
    /// The 32-byte Ed25519 seed underlying the keypair. Under ADR-039
    /// this value is stable across passphrase rotation; on legacy
    /// projects (no seed_wrap_*) it equals the Argon2id-derived KEK.
    pub seed: [u8; 32],
}

/// Verify a passphrase against a member entry and return the resulting
/// `IdentityKeypair` and seed. Handles both the wrapped-seed model
/// (ADR-039) and the legacy model where the Argon2id-derived key is the
/// Ed25519 seed.
///
/// Does **not** perform lazy migration; that is `auth_with_passphrase`'s
/// responsibility on the human-facing `joy auth` path. Other commands
/// that need the acting human's keypair (`joy ai init`, `joy auth
/// reset`, `joy auth token add`, `joy ai rotate`) call this helper and
/// let the next `joy auth` migrate.
pub fn unlock_identity(
    member: &crate::model::project::Member,
    passphrase: &str,
) -> Result<UnlockedIdentity, JoyError> {
    let salt_hex = member
        .kdf_nonce
        .as_ref()
        .ok_or_else(|| JoyError::AuthFailed("member has no kdf_nonce".into()))?;
    let salt = Salt::from_hex(salt_hex)?;
    let verify_hex = member
        .verify_key
        .as_ref()
        .ok_or_else(|| JoyError::AuthFailed("member has no verify_key".into()))?;
    let verify_key = PublicKey::from_hex(verify_hex)?;

    let (kp, seed_bytes) = if let Some(wrap_hex) = member.seed_wrap_passphrase.as_deref() {
        let seed = seed::unwrap_seed_with_passphrase(wrap_hex, passphrase, &salt)?;
        let bytes = *seed.as_bytes();
        (IdentityKeypair::from_seed(&bytes), bytes)
    } else {
        let key = derive_key(passphrase, &salt)?;
        let bytes = *key.as_bytes();
        (IdentityKeypair::from_derived_key(&key), bytes)
    };
    if kp.public_key() != verify_key {
        return Err(JoyError::AuthFailed("incorrect passphrase".into()));
    }
    Ok(UnlockedIdentity {
        keypair: kp,
        seed: seed_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
