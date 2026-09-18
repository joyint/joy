// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The shared half of the Joy forge connectors (JOY-0298-E4, design
//! `docs/design/forge-connection-ng.md`, package J2).
//!
//! One binary, `joy-forge`, carries every forge (D2.1); the forge
//! knowledge stays in the per forge crates and everything they share
//! lives here: the command line and the protocol (D2.2, D2.2a), the in
//! process HTTP client with the proxy sources of D1.11 and the trust
//! store of D1.12 (D2.8), the instance configuration of D2.5, the
//! foreign CLI discovery of D2.4 and the scope sets of D2.7a and D2.7c.
//!
//! Since JOY-029B-B0 (package J3) the sign in half lives here too:
//! [`auth`] is the connector's own credential entry, the refresh lock
//! of D2.6a, the two OAuth doors of D2.7 and the login order of D4.1c.
//!
//! Nothing here knows a forge. A forge is a [`forge::Forge`]
//! implementation the binary hands to [`cli::run`].

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
