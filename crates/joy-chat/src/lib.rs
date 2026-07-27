// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The encrypted chat subsystem for Joy (human and AI chat), split out of
//! joy-core per ADR-043. Sits above joy-core, below joy-ai.

#![deny(clippy::all)]

pub mod chat_events;
pub mod chat_ref;
pub mod chat_seal;
pub mod chat_state;
pub mod chat_store;
pub mod chat_wrap;
pub mod chats;
pub mod mentions;
pub mod model;
pub mod turn_meta;
pub mod writer;
