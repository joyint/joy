// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The shared half of the Joy forge connectors (JOY-0298-E4, the forge
//! connection design).
//!
//! One binary, `joy-forge`, carries every forge; the forge
//! knowledge stays in the per forge crates and everything they share
//! lives here: the command line and the protocol, the in
//! process HTTP client with the proxy sources and the trust
//! store, the instance configuration, the
//! foreign CLI discovery and the scope sets.
//!
//! Since JOY-029B-B0 the sign in half lives here too:
//! [`auth`] is the connector's own credential entry, the refresh lock,
//! the two OAuth doors and the login order.
//!
//! Nothing here knows a forge. A forge is a [`forge::Forge`]
//! implementation the binary hands to [`cli::run`].

// Nothing of the developer's shell and session reaches the unit tests
// of this crate (JOY-02BB-C7).
#[cfg(test)]
joy_test_env::isolate!();

pub mod auth;
pub mod cli;
pub mod config;
#[cfg(feature = "fake-api")]
pub mod fake;
pub mod foreign;
pub mod forge;
pub mod gitconfig;
pub mod http;
pub mod proxy;
pub mod scope;
pub mod trust;
pub mod url;

pub use auth::{ChoseBy, Purpose, Resolved, Source};
pub use forge::{
    Account, Ctx, Forge, HostKind, Listing, NewRepository, Reach, ReleaseRequest, Target,
    DEFAULT_LIMIT,
};
pub use http::{Answer, Http, HttpError};
