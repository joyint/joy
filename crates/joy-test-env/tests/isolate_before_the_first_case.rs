// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! `isolate!` runs when the binary loads (JOY-02BB-C7). This binary is
//! the proof: its one case does nothing but look, so what it finds was
//! done before it ran. `just test-unit` and CI run it on every system
//! joy builds for, which is where the loader's section name is checked.

joy_test_env::isolate!();

#[test]
fn the_process_was_swept_before_the_first_case_ran() {
    assert_eq!(
        std::env::var_os(joy_test_env::SESSION_BUS).as_deref(),
        Some(std::ffi::OsStr::new(joy_test_env::NO_SESSION_BUS)),
        "the loader ran isolate! ahead of main"
    );
    assert!(
        joy_test_env::inherited_state().is_empty(),
        "nothing of the person's shell is left: {:?}",
        joy_test_env::inherited_state()
    );
    let path = std::env::var_os("PATH").expect("a PATH");
    assert_eq!(
        std::env::split_paths(&path).next().as_deref(),
        Some(joy_test_env::forge_cli_dir()),
        "the forge CLI stand-ins come first"
    );
}
