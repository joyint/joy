// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Per-(operator, AI) delegation key derivation — re-exported from the
//! wasm-portable `joy-token` crate (JI-0175-B0) so the browser derives the
//! delegation keypair with the exact same HKDF the CLI and platform use.

pub use joy_token::delegation::*;
