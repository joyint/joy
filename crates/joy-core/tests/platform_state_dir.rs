// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! End-to-end check that the native (non-XDG) state-directory branch of
//! `dirs_state_dir()` resolves to a usable, writable location and that a
//! real session save/load roundtrip works through it. See JOY-01A2-96.
//!
//! The in-crate unit `save_load_roundtrip` exercises the XDG branch (it sets
//! `XDG_STATE_HOME`), which is identical code on every platform. This test
//! deliberately clears `XDG_STATE_HOME` so the *platform-native* branch runs:
//! `$HOME/.local/state` on Unix, `%LOCALAPPDATA%` on Windows. On a
//! `windows-latest` CI runner it therefore proves the Windows branch end to
//! end (cfg!(windows) routing + real file I/O with backslash paths); on Unix
//! it proves the `$HOME/.local/state` path. It also guards against the
//! "silent fallback to `.`" regression by asserting the session file lands
//! under the controlled temp dir, never the current working directory.
//!
//! Lives in `tests/` (its own test binary, i.e. its own process) so its
//! env-var mutation cannot race the in-crate unit test that sets
//! `XDG_STATE_HOME`. It is the only env-mutating test in this binary.

use joy_core::auth::session::{self, create_session};
use joy_core::auth::{derive_key, IdentityKeypair, PublicKey, Salt};

fn test_keypair() -> (IdentityKeypair, PublicKey) {
    let salt =
        Salt::from_hex("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef").unwrap();
    let key = derive_key("correct horse battery staple extra words", &salt).unwrap();
    let kp = IdentityKeypair::from_derived_key(&key);
    let pk = kp.public_key();
    (kp, pk)
}

/// Recursively search `dir` for any `*.json` file; returns true if one exists.
fn contains_json(dir: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if contains_json(&path) {
                return true;
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
            return true;
        }
    }
    false
}

#[test]
fn native_state_dir_roundtrip_lands_under_home_base() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().to_path_buf();

    // Save the env we are about to mutate, restore it on the way out so other
    // test binaries (and the developer's shell) are unaffected.
    let saved_xdg = std::env::var_os("XDG_STATE_HOME");
    let saved_home = std::env::var_os("HOME");
    let saved_local = std::env::var_os("LOCALAPPDATA");

    // SAFETY: single env-mutating test in its own test binary (process);
    // no other thread in this binary touches these vars.
    unsafe {
        // Force the native branch: no XDG override.
        std::env::remove_var("XDG_STATE_HOME");
        // Drive both platform bases to the same controlled temp dir so the
        // assertion holds regardless of host OS:
        //   Unix    -> $HOME/.local/state
        //   Windows -> %LOCALAPPDATA%
        std::env::set_var("HOME", &base);
        std::env::set_var("LOCALAPPDATA", &base);
    }

    let (kp, pk) = test_keypair();
    let token = create_session(&kp, "test@example.com", "TST", None);

    session::save_session("TST", &token).expect("save_session must succeed on the native path");
    let loaded = session::load_session("TST", "test@example.com")
        .expect("load_session must not error")
        .expect("a session must have been written");
    let claims = session::validate_session(&loaded, &pk, "TST").expect("loaded session validates");
    assert_eq!(claims.member, "test@example.com");

    // The file must live under our controlled base dir -- proves the native
    // branch was used and that it did NOT silently fall back to ".".
    assert!(
        contains_json(&base),
        "session file should have been written under the native base dir {base:?}"
    );

    session::remove_session("TST", "test@example.com").unwrap();
    assert!(session::load_session("TST", "test@example.com")
        .unwrap()
        .is_none());

    // SAFETY: restore prior env; same single-thread justification as above.
    unsafe {
        match saved_xdg {
            Some(v) => std::env::set_var("XDG_STATE_HOME", v),
            None => std::env::remove_var("XDG_STATE_HOME"),
        }
        match saved_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match saved_local {
            Some(v) => std::env::set_var("LOCALAPPDATA", v),
            None => std::env::remove_var("LOCALAPPDATA"),
        }
    }
}
