// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Per-(operator, AI) delegation key derivation.
//!
//! Each operator has one stable Ed25519 delegation keypair per AI member
//! they delegate to. The keypair is derived deterministically via HKDF
//! from the operator's identity seed, a per-(operator, AI)
//! `delegation_salt` recorded in `project.yaml`, and the (project, AI)
//! identifier used as the HKDF info parameter.
//!
//! The matching public key (`delegation_verifier`) is recorded once in
//! `project.yaml` under
//! `members.<operator>.ai_delegations.<ai-member>`. The private key is
//! re-derived in process memory whenever the operator enters their
//! passphrase, used for the signing or wrap operation, and discarded.
//! It is never persisted to disk; rotating the salt is the explicit
//! revocation handle.

use joy_crypt::kdf::{derive_hkdf_sha256, Salt};

/// Derive a 32-byte Ed25519 seed for a per-(operator, AI) delegation
/// key.
///
/// Inputs:
///   - `identity_seed`: the operator's 32-byte identity seed (the value
///     unwrapped from `seed_wrap_passphrase`, stable across passphrase
///     rotation).
///   - `salt`: the per-(operator, AI) `delegation_salt` recorded in
///     `project.yaml`.
///   - `project_id`: the canonical project id (acronym today).
///   - `ai_member_id`: the AI member id, e.g. `ai:claude@joy`.
///
/// HKDF-SHA256 is used in extract-and-expand form. The `info` parameter
/// embeds project and member ids so the same `(seed, salt)` cannot be
/// replayed across (project, AI) pairs.
pub fn derive_delegation_seed(
    identity_seed: &[u8; 32],
    salt: &Salt,
    project_id: &str,
    ai_member_id: &str,
) -> [u8; 32] {
    let mut info = Vec::with_capacity(16 + project_id.len() + 1 + ai_member_id.len());
    info.extend_from_slice(b"joy-delegation:");
    info.extend_from_slice(project_id.as_bytes());
    info.push(b':');
    info.extend_from_slice(ai_member_id.as_bytes());
    derive_hkdf_sha256(identity_seed, salt.as_bytes(), &info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use joy_crypt::kdf::generate_salt;

    const FIXED_SEED: [u8; 32] = [7u8; 32];

    #[test]
    fn delegation_seed_is_deterministic() {
        let salt = generate_salt();
        let s1 = derive_delegation_seed(&FIXED_SEED, &salt, "JOY", "ai:claude@joy");
        let s2 = derive_delegation_seed(&FIXED_SEED, &salt, "JOY", "ai:claude@joy");
        assert_eq!(s1, s2);
    }

    #[test]
    fn delegation_seed_changes_with_salt() {
        let s1 = derive_delegation_seed(&FIXED_SEED, &generate_salt(), "JOY", "ai:claude@joy");
        let s2 = derive_delegation_seed(&FIXED_SEED, &generate_salt(), "JOY", "ai:claude@joy");
        assert_ne!(s1, s2);
    }

    #[test]
    fn delegation_seed_is_domain_separated_by_project() {
        let salt = generate_salt();
        let s1 = derive_delegation_seed(&FIXED_SEED, &salt, "JOY", "ai:claude@joy");
        let s2 = derive_delegation_seed(&FIXED_SEED, &salt, "OTHER", "ai:claude@joy");
        assert_ne!(s1, s2);
    }

    #[test]
    fn delegation_seed_is_domain_separated_by_member() {
        let salt = generate_salt();
        let s1 = derive_delegation_seed(&FIXED_SEED, &salt, "JOY", "ai:claude@joy");
        let s2 = derive_delegation_seed(&FIXED_SEED, &salt, "JOY", "ai:qwen@joy");
        assert_ne!(s1, s2);
    }

    #[test]
    fn delegation_seed_changes_with_identity_key() {
        let salt = generate_salt();
        let seed_a = FIXED_SEED;
        let seed_b: [u8; 32] = [8u8; 32];
        let s1 = derive_delegation_seed(&seed_a, &salt, "JOY", "ai:claude@joy");
        let s2 = derive_delegation_seed(&seed_b, &salt, "JOY", "ai:claude@joy");
        assert_ne!(s1, s2);
    }
}
