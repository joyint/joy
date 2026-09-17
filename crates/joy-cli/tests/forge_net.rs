// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The packaging fact of D3.1: the libgit2 this binary links speaks
//! https and ssh.
//!
//! Until the git2 only move the CLI reached a forge by spawning `git`,
//! so the transports libgit2 was built with did not matter here and
//! joy-cli left `joy-core/forge-net` off. Now every CLI contact travels
//! through libgit2, and a build without the transports answers each of
//! them with "unsupported URL protocol" - a refusal the classifier can
//! only read as `error` and no person can act on.
//!
//! This is a property of the LINKED library, not of a manifest line, so
//! it is asserted where it is felt. Reading the manifest instead would
//! pass on a machine where cargo resolved the feature away.

/// joy's own binary is built from this crate, so the libgit2 a test
/// binary links is the one `joy` links: same workspace, same unified
/// feature set for git2.
#[test]
fn the_cli_links_a_libgit2_that_can_reach_a_forge() {
    let version = git2::Version::get();
    assert!(
        version.https(),
        "libgit2 was built without a TLS backend, so every https remote answers \
         'unsupported URL protocol' (D3.1: joy-cli must enable joy-core/forge-net)"
    );
    assert!(
        version.ssh(),
        "libgit2 was built without libssh2, so every ssh remote answers \
         'unsupported URL protocol' (D3.1: joy-cli must enable joy-core/forge-net)"
    );
}

/// The same fact read from the engine's side: joy-core's own transport
/// probe agrees with the linked library, so a caller that asks joy-core
/// "can you reach this remote" gets the true answer here.
#[test]
fn the_engine_agrees_that_the_transports_are_there() {
    assert!(
        joy_core::vcs::forge::transports_available(),
        "joy-core reports no network transports in a joy-cli build (D3.1)"
    );
}
