// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! What can go wrong while opening or sealing a chat.
//!
//! A short list on purpose. This crate reads and writes bytes it was
//! handed, so there is no store to be missing, no repository to be dirty
//! and no network to be down. `joy-core`'s big `JoyError` covers those and
//! converts from this one, so a caller on the storage side keeps writing
//! `?` exactly as before.

/// A failure while opening or sealing chat content.
#[derive(Debug, thiserror::Error)]
pub enum ChatError {
    /// The material at hand does not open what it should: a wrong seed, a
    /// slot for someone else, a tampered blob. Never says WHICH, because
    /// the caller must not learn that either.
    #[error("{0}")]
    Auth(String),
    /// Content that does not parse as what it claims to be.
    #[error("{0}")]
    Format(String),
    /// Event YAML that does not serialize or parse.
    #[error("chat event: {0}")]
    Yaml(#[from] serde_yaml_ng::Error),
}

impl ChatError {
    /// Shorthand for the auth case, which is most of them.
    pub fn auth(what: impl Into<String>) -> Self {
        Self::Auth(what.into())
    }
}
