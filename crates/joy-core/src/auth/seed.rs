// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Wrapped-seed identity helpers (ADR-039). The implementation moved into
//! joy-crypt (the pure, wasm-portable crypto crate) so browser clients can
//! run the exact same derivation (JP-004B); joy-core re-exports it here to
//! keep the `auth::seed` path stable for existing callers.

pub use joy_crypt::seed::{
    unwrap_seed_with_passphrase, unwrap_seed_with_recovery, wrap_seed_for_migration,
    wrap_seed_with_passphrase, wrap_seed_with_recovery, RecoveryKey, Seed,
};
