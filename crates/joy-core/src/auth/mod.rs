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
pub mod enroll;
pub mod grants;
pub mod login;
pub mod otp;
pub mod redeem;
pub mod seed;
pub mod session;
pub mod token;
pub mod vouch;

// Re-export joy-crypt primitives under joy-domain names. Callers within
// joy-core/auth and joy-cli use these names; the underlying
// implementation lives in joy-crypt (ADR-039).
pub use joy_crypt::identity::{Keypair as IdentityKeypair, PublicKey};
pub use joy_crypt::kdf::{derive_argon2id as derive_key, generate_salt, DerivedKey, Salt};

use crate::error::JoyError;

/// Whether a passphrase may be set at all (JI-01A5-93): only an empty
/// one is refused. Its strength is shown while it is typed, by
/// `joy_passphrase::strength`, and never enforced.
pub fn accept_passphrase(passphrase: &str) -> Result<(), JoyError> {
    joy_passphrase::accept(passphrase).map_err(|_| JoyError::PassphraseEmpty)
}

/// What a NEW passphrase must be (JOY-02CD-14): accepted at all, and not
/// the current one. The CLI asks this before its confirmation prompt,
/// [`change_passphrase`] asks it for the desktop.
pub fn check_new_passphrase(current: &str, new: &str) -> Result<(), JoyError> {
    accept_passphrase(new)?;
    if new == current {
        return Err(JoyError::PassphraseUnchanged);
    }
    Ok(())
}

/// Change a member's passphrase (JOY-02CD-14): the current one opens the
/// seed, the same seed is wrapped under the new one with the same
/// `kdf_nonce`; the recovery wrap and the verify key stay. The caller
/// saves the project. A member without a seed wrap (the legacy schema)
/// is refused here; `joy auth passphrase` migrates such an entry first.
pub fn change_passphrase(
    project: &mut crate::model::Project,
    member_key: &str,
    current: &str,
    new: &str,
) -> Result<(), JoyError> {
    check_new_passphrase(current, new)?;
    let member = project
        .member_by_key(member_key)
        .ok_or_else(|| JoyError::AuthFailed(format!("{member_key} is not a project member")))?;
    let salt_hex = member
        .kdf_nonce
        .as_deref()
        .ok_or_else(|| JoyError::AuthFailed(format!("{member_key} has no identity key yet")))?;
    let wrap_hex = member
        .seed_wrap
        .as_deref()
        .ok_or_else(|| JoyError::AuthFailed(format!("{member_key} has no passphrase wrap")))?;
    let salt = Salt::from_hex(salt_hex)?;
    let new_wrap = joy_crypt::seed::rewrap_seed_with_passphrase(wrap_hex, current, new, &salt)?;
    project
        .member_by_key_mut(member_key)
        .expect("found above")
        .seed_wrap = Some(new_wrap);
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

    let (kp, seed_bytes) = if let Some(wrap_hex) = member.seed_wrap.as_deref() {
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
    fn a_new_passphrase_is_any_text_but_the_current_one() {
        assert!(matches!(
            check_new_passphrase("q w e", ""),
            Err(JoyError::PassphraseEmpty)
        ));
        assert!(matches!(
            check_new_passphrase("q w e", "q w e"),
            Err(JoyError::PassphraseUnchanged)
        ));
        assert!(check_new_passphrase("q w e", "x").is_ok());
    }

    #[test]
    fn change_passphrase_moves_the_seed_and_keeps_the_identity() {
        use crate::model::project::{Member, MemberCapabilities, Project};
        use joy_crypt::seed::{unwrap_seed_with_passphrase, wrap_seed_with_passphrase, Seed};
        let mut project = Project::new("Change".into(), Some("CHG".into()));
        let salt = generate_salt();
        let seed = Seed::generate();
        let keypair = IdentityKeypair::from_seed(seed.as_bytes());
        let mut member = Member::person(MemberCapabilities::All);
        member.kdf_nonce = Some(salt.to_hex());
        member.verify_key = Some(keypair.public_key().to_hex());
        member.seed_wrap = Some(wrap_seed_with_passphrase(&seed, "old one", &salt).unwrap());
        project.register_member("a@b.c", member).unwrap();

        change_passphrase(&mut project, "a@b.c", "old one", "new one").unwrap();

        let wrap = project
            .member_by_key("a@b.c")
            .unwrap()
            .seed_wrap
            .clone()
            .unwrap();
        let opened = unwrap_seed_with_passphrase(&wrap, "new one", &salt).unwrap();
        assert_eq!(opened.as_bytes(), seed.as_bytes());
        assert!(unwrap_seed_with_passphrase(&wrap, "old one", &salt).is_err());
        assert_eq!(
            project
                .member_by_key("a@b.c")
                .unwrap()
                .verify_key
                .as_deref(),
            Some(keypair.public_key().to_hex().as_str())
        );
        // a wrong current passphrase changes nothing
        assert!(change_passphrase(&mut project, "a@b.c", "old one", "third").is_err());
        assert!(unwrap_seed_with_passphrase(&wrap, "new one", &salt).is_ok());
    }

    #[test]
    fn only_the_empty_passphrase_is_refused() {
        assert!(matches!(
            accept_passphrase(""),
            Err(JoyError::PassphraseEmpty)
        ));
        assert!(accept_passphrase("q").is_ok());
        assert!(accept_passphrase("q w e").is_ok());
        assert!(accept_passphrase("one two three four five six").is_ok());
    }
}
