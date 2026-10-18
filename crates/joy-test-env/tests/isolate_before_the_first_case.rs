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
    // The store of every entry is keyring's mock, on every system, so
    // nothing here reaches the operating system's credential store; and
    // like that mock, it persists nothing past the entry written through.
    let entry = keyring::Entry::new("joy-test-env", "probe").expect("an entry");
    assert!(
        entry
            .get_credential()
            .downcast_ref::<keyring::mock::MockCredential>()
            .is_some(),
        "the loader made keyring's mock the default store"
    );
    entry.set_password("written").expect("the mock takes it");
    assert!(
        keyring::Entry::new("joy-test-env", "probe")
            .expect("a second entry")
            .get_password()
            .is_err(),
        "a second entry finds nothing"
    );
}
