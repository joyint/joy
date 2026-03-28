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

pub mod derive;
pub mod session;
pub mod sign;
