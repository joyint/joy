// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! joy follows git's own switch for the system config (JOY-028D-46).
//!
//! libgit2 always reads the system-wide gitconfig; git skips it when
//! `GIT_CONFIG_NOSYSTEM` is true. A script that isolates HOME and sets the
//! variable must find the same identity in joy as in git: none. The nightly
//! runner has an identity in its /etc/gitconfig, and joy init found it.
//!
//! Its own test binary: it changes the process environment, the working
//! directory and libgit2's search path, all process state.

use std::fs;

use joy_core::vcs::forge;

#[test]
fn the_system_identity_is_read_unless_git_config_nosystem_says_otherwise() {
    let home = tempfile::tempdir().unwrap();
    let system = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(
        system.path().join("gitconfig"),
        "[user]\n\temail = system@example.test\n\tname = System\n",
    )
    .unwrap();
    // no global, no XDG config, no repository around the working directory
    std::env::set_var("HOME", home.path());
    std::env::set_var("XDG_CONFIG_HOME", home.path().join(".config"));
    std::env::remove_var("GIT_CONFIG_NOSYSTEM");
    std::env::set_current_dir(outside.path()).unwrap();

    // the first look settles joy's view of the variable (unset)...
    forge::user_email();
    // ...then every other config level is the empty home and the system
    // config the temporary one. The levels are set on libgit2 itself, not
    // through the environment: on Windows the global config is also looked
    // up under USERPROFILE, where the CI runner keeps its own identity.
    unsafe {
        for level in [
            git2::ConfigLevel::Global,
            git2::ConfigLevel::XDG,
            git2::ConfigLevel::ProgramData,
        ] {
            git2::opts::set_search_path(level, home.path().to_str().unwrap()).unwrap();
        }
        git2::opts::set_search_path(git2::ConfigLevel::System, system.path().to_str().unwrap())
            .unwrap();
    }
    assert_eq!(forge::user_email().as_deref(), Some("system@example.test"));

    // git's switch: the system config is not read
    std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    assert_eq!(forge::user_email(), None);

    // a false value reads the system config again, as git does: the look
    // that notices the change restores libgit2's own search path, which is
    // then pointed at the temporary system config once more
    std::env::set_var("GIT_CONFIG_NOSYSTEM", "false");
    forge::user_email();
    unsafe {
        git2::opts::set_search_path(git2::ConfigLevel::System, system.path().to_str().unwrap())
            .unwrap();
    }
    assert_eq!(forge::user_email().as_deref(), Some("system@example.test"));
}
