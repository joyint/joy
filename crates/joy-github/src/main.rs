// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! `joy-github`: the legacy name of the GitHub connector.
//!
//! One binary carries every forge now (D2.1). This name stays as a PATH
//! fallback for people who ran `cargo install joy-github`, for the one
//! deprecation window of D2.2a, and it is the same code: the protocol
//! and every verb come from `joy_forge_net::cli`, with a list of one
//! forge instead of three.

use joy_forge_net::cli::{self, Manifest};
use joy_forge_net::forge::Forge;

const MANIFEST: Manifest = Manifest {
    name: "joy-github",
    version: env!("CARGO_PKG_VERSION"),
};

fn main() {
    let forges: Vec<&dyn Forge> = vec![&joy_github::FORGE];
    std::process::exit(cli::run(&forges, &MANIFEST));
}
