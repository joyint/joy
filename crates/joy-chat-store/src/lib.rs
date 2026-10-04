// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Where a sealed chat LIVES: git blobs, trees, and the chats ref.
//!
//! Split out of `joy-chat` (JAPP-0135-FD). The other half describes what a
//! chat is and can open and seal one; this half knows nothing about keys
//! and everything about storage. The cut is the reason the app can run the
//! chat itself in the browser while the git work stays where a git
//! repository is: on the desktop, on the server, in the CLI.

#![deny(clippy::all)]

// Nothing of the developer's shell and session reaches the unit tests
// of this crate (JOY-02BB-C7).
#[cfg(test)]
joy_test_env::isolate!();

pub mod chat_ref;
pub mod chat_state;
pub mod chat_store;
pub mod chats;
pub mod migrations;
pub mod turn_meta;
pub mod update;
pub mod writer;
