// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The acceptance sentence of J4a, run: on a machine with
//! `credential.helper=manager` and NO git on PATH, joy obtains a
//! credential, and nothing in the run is a git process (forge
//! connection NG, design D1.3).
//!
//! git2's own runner cannot do this. It builds the string
//! `git credential-<name>` for every short helper name and hands it to
//! `sh -c` (cred.rs:310-316, :395), so on a machine with no git binary
//! it fails, and on Windows it fails again for `sh`. joy resolves the
//! helper to a binary called `git-credential-<name>` and spawns that
//! binary by its own name.
//!
//! Its own test binary: it moves PATH, which is process state.

#![cfg(unix)]

use joy_core::vcs::credential_helper::{self, Search};
use joy_core::vcs::HostKind;

fn executable(path: &std::path::Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn a_credential_arrives_on_a_machine_with_no_git_on_path() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let ran = dir.path().join("ran");
    executable(
        &bin.join("git-credential-manager"),
        &format!(
            "echo \"$1\" >> {ran}\nif [ \"$1\" = get ]; then echo username=x-access-token; echo password=s3cret; fi",
            ran = ran.display()
        ),
    );
    // The whole machine, as far as this process can see it: one
    // directory, holding the helper and no git.
    std::env::set_var("PATH", &bin);
    assert!(
        !bin.join("git").exists(),
        "the point of this test is a machine without git"
    );
    let search = Search::of_this_machine();
    assert!(
        search
            .dirs
            .iter()
            .all(|found| !found.join("git").is_file() && !found.join("git.exe").is_file()),
        "no git anywhere joy would look: {:?}",
        search.dirs
    );

    let config_path = dir.path().join("gitconfig");
    std::fs::write(&config_path, "[credential]\n\thelper = manager\n").unwrap();
    let config = git2::Config::open(&config_path).unwrap();

    credential_helper::forget_all();
    let credential = credential_helper::get(
        &config,
        "https://github.com/joyint/joy.git",
        None,
        HostKind::Background,
    )
    .expect("the helper answered")
    .expect("a credential");
    assert_eq!(credential.username, "x-access-token");
    assert_eq!(credential.password, "s3cret");
    assert_eq!(credential.helper, "manager");

    // The other half of D1.3 that git2 never runs: the outcome goes
    // back to the helper, so a revoked entry is erased instead of
    // replayed on the next contact.
    credential_helper::accepted("https://github.com/joyint/joy.git");
    assert_eq!(
        std::fs::read_to_string(&ran)
            .unwrap()
            .split_whitespace()
            .collect::<Vec<_>>(),
        vec!["get", "store"]
    );
    credential_helper::forget_all();
}
