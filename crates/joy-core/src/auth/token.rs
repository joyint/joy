// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! AI delegation tokens — re-exported from the wasm-portable `joy-token`
//! crate (JI-0175-B0).
//!
//! The token machinery lives in `joy-token` so the CLI, the platform, and
//! the browser run the exact same code with no duplicated claims struct.
//! joy-core keeps the historical `joy_core::auth::token::*` path and maps
//! `joy_token::TokenError` into [`crate::error::JoyError`] (see the
//! `From` impl in `crate::error`).

pub use joy_token::token::*;
