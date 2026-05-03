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
pub mod session;
pub mod token;

// Re-export joy-crypt primitives under joy-domain names. Callers within
// joy-core/auth and joy-cli use these names; the underlying
// implementation lives in joy-crypt (ADR-039).
pub use joy_crypt::identity::{Keypair as IdentityKeypair, PublicKey};
pub use joy_crypt::kdf::{
    derive_argon2id as derive_key, generate_salt, DerivedKey, Salt,
};

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

/// Cross-module test lock: modules in this tree mutate process-global
/// `XDG_STATE_HOME` in their unit tests. Cargo runs tests in parallel, so
/// without one shared mutex the modules would trample each other's
/// per-test tempdir overrides.
#[cfg(test)]
pub(super) static STATE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
