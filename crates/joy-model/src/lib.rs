// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The few types that everything in Joy shares, kept in a crate that has
//! no ties to a file system, a git repository or an operating system.
//!
//! That is the whole reason this crate exists: `joy-chat` describes what a
//! chat IS and must compile for the browser (JAPP-0135-FD), while these
//! types come from `joy-core`, which never will because it carries git.
//! Moving them down here is the smallest cut that frees the chat crate,
//! and `joy-core` re-exports them at their old paths, so nothing else
//! changes.

#![forbid(unsafe_code)]

// Nothing of the developer's shell and session reaches the unit tests
// of this crate (JOY-02BB-C7).
#[cfg(test)]
joy_test_env::isolate!();

pub mod interaction;
pub mod member_ref;

pub use interaction::InteractionLevel;
pub use member_ref::MemberRef;
