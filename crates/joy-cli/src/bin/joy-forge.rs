// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! `joy-forge`: the one connector binary (design D2.1, package J2).
//!
//! Three separate connectors cost 3.3 MB stripped, of which about 3 MB
//! was a duplicated std, clap and serde floor. One binary carrying all
//! three forges is the size of one of them, and it is a `[[bin]]` of
//! the package that already produces `joy`: one archive, one installer
//! change, one receipt and one sidecar per platform cover everything.
//!
//! The forge is the first argument (`joy-forge github claims ...`);
//! `version` is the binary's own question and takes none (D2.2a).

use joy_forge_net::cli::{self, Manifest};
use joy_forge_net::forge::Forge;

const MANIFEST: Manifest = Manifest {
    name: "joy-forge",
    version: env!("CARGO_PKG_VERSION"),
};

fn main() {
    let forges: Vec<&dyn Forge> = vec![&joy_github::FORGE, &joy_gitlab::FORGE, &joy_gitea::FORGE];
    std::process::exit(cli::run(&forges, &MANIFEST));
}
