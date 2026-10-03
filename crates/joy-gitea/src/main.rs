// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! `joy-gitea`: the legacy name of the Gitea connector.
//!
//! One binary carries every forge now. This name stays as a PATH
//! fallback for people who ran `cargo install joy-gitea`, for the one
//! deprecation window, and it is the same code.

use joy_forge_net::cli::{self, Manifest};
use joy_forge_net::forge::Forge;

const MANIFEST: Manifest = Manifest {
    name: "joy-gitea",
    version: env!("CARGO_PKG_VERSION"),
};

fn main() {
    let forges: Vec<&dyn Forge> = vec![&joy_gitea::FORGE];
    std::process::exit(cli::run(&forges, &MANIFEST));
}
