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
//! Passphrase + Salt --[Argon2id]--> DerivedKey --[Ed25519]--> IdentityKeypair
//! ```

pub mod delegation;
pub mod derive;
pub mod session;
pub mod sign;
pub mod token;

/// Cross-module test lock: modules in this tree mutate process-global
/// `XDG_STATE_HOME` in their unit tests. Cargo runs tests in parallel, so
/// without one shared mutex the modules would trample each other's
/// per-test tempdir overrides.
#[cfg(test)]
pub(super) static STATE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
