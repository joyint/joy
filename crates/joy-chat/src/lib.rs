// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! What a chat IS: its model, its events, and the crypto that seals them.
//!
//! Everything here is pure. No git, no file system, no network, so the same
//! code runs natively for the CLI and as WebAssembly in the app's webview
//! (JAPP-0135-FD). That is the whole point: a chat sealed on a laptop and a
//! chat opened in a browser go through one implementation, byte for byte,
//! and cannot drift apart.
//!
//! Storage lives next door in `joy-chat-store`, which moves these bytes in
//! and out of a git repository and never sees a key.

#![deny(clippy::all)]
#![forbid(unsafe_code)]

pub mod chat_events;
pub mod chat_seal;
pub mod chat_wrap;
pub mod error;
pub mod mentions;
pub mod model;
pub mod sealed;
pub mod turns;

pub use error::ChatError;
