// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The capture guard on the founding address (JOY-0253-8A, D4.4 of the
//! forge connection NG design): a forge alias address must never become a
//! member key, not even when a person names it themselves.
//!
//! Decided here, once: `--user` and the terminal ask are explicit
//! overrides and are refused all the same, because the split identity the
//! alias produces is the same whoever typed it, and the person cannot see
//! that their forge handed them an alias. The surfaces that OFFER
//! addresses filter aliases out before showing them (D4.4), so this
//! refusal is only ever met by someone who typed one.
//!
//! Its own test binary: it puts a plugin stub on the PATH, which is
//! process state. Unix only, because the stub is a shell script; the rule
//! it proves is platform independent and the other cases cover it.
#![cfg(unix)]

use std::path::Path;
use std::sync::OnceLock;

use joy_core::error::JoyError;
use joy_core::host::HostKind;
use joy_core::init::{self, InitOptions, TerminalAsk};

const ALIAS: &str = "1234+scotty@users.noreply.example.com";
const REAL: &str = "scotty@example.com";

/// A `joy-github` on the PATH that claims every remote and calls exactly
/// one address an alias, on a machine that has no git identity of its own
/// (otherwise the ask below is never reached). Both are process state, so
/// both are applied once for this binary.
fn plugin_stub_on_the_path() {
    static STUB: OnceLock<(tempfile::TempDir, tempfile::TempDir)> = OnceLock::new();
    STUB.get_or_init(|| {
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());
        std::env::set_var("XDG_CONFIG_HOME", home.path().join(".config"));
        std::env::set_var("XDG_STATE_HOME", home.path().join(".state"));
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        std::env::set_current_dir(home.path()).unwrap();
        joy_core::vcs::forge::user_email();
        unsafe {
            for level in [
                git2::ConfigLevel::Global,
                git2::ConfigLevel::XDG,
                git2::ConfigLevel::ProgramData,
            ] {
                git2::opts::set_search_path(level, home.path().to_str().unwrap()).unwrap();
            }
        }
        assert_eq!(joy_core::vcs::forge::user_email(), None);

        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("joy-github");
        std::fs::write(
            &binary,
            format!(
                r#"#!/bin/sh
case "$1" in
claims) echo '{{"claims":true}}' ;;
resolve)
    shift
    while [ "$1" != "--email" ] && [ -n "$1" ]; do shift; done
    if [ "$2" = "{ALIAS}" ]; then
        echo '{{"known":true,"login":"scotty","emails":["{REAL}"]}}'
    else
        echo '{{"known":false}}'
    fi
    ;;
*) echo '{{"known":false}}' ;;
esac
"#
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", dir.path().to_str().unwrap(), path));
        (dir, home)
    });
}

/// A checkout with a github remote, so the stub is the responsible plugin.
fn checkout_with_a_github_remote(root: &Path) {
    let repo = git2::Repository::init(root).unwrap();
    repo.remote("origin", "https://github.example.com/scotty/ship.git")
        .unwrap();
}

/// The guard refuses the alias even though the person named it with
/// `--user`, and leaves no project behind.
#[test]
fn an_explicitly_named_alias_is_refused() {
    plugin_stub_on_the_path();
    let dir = tempfile::tempdir().unwrap();
    checkout_with_a_github_remote(dir.path());

    let err = init::init(InitOptions {
        name: Some("Alias".into()),
        user: Some(ALIAS.to_string()),
        ..InitOptions::new(dir.path().to_path_buf())
    })
    .unwrap_err();

    assert!(
        matches!(err, JoyError::FounderAliasIdentity(ref a) if a == ALIAS),
        "{err}"
    );
    assert!(err.to_string().contains("is a forge alias address"));
    assert!(!dir.path().join(".joy").exists());
}

/// An address a person types at the ask is an override like any other and
/// meets the same guard.
#[test]
fn an_alias_typed_at_the_ask_is_refused_too() {
    plugin_stub_on_the_path();
    let dir = tempfile::tempdir().unwrap();
    checkout_with_a_github_remote(dir.path());

    let err = init::init(InitOptions {
        name: Some("Typed".into()),
        host: HostKind::Interactive,
        ask: Some(Box::new(TerminalAsk::new(
            std::io::Cursor::new(format!("{ALIAS}\n").into_bytes()),
            Vec::new(),
        ))),
        ..InitOptions::new(dir.path().to_path_buf())
    })
    .unwrap_err();

    assert!(
        matches!(err, JoyError::FounderAliasIdentity(ref a) if a == ALIAS),
        "{err}"
    );
    assert!(!dir.path().join(".joy").exists());
}

/// The guard refuses aliases, not addresses: the person's real one founds
/// the project with the same plugin answering.
#[test]
fn the_real_address_founds_the_project() {
    plugin_stub_on_the_path();
    let dir = tempfile::tempdir().unwrap();
    checkout_with_a_github_remote(dir.path());

    let result = init::init(InitOptions {
        name: Some("Real".into()),
        user: Some(REAL.to_string()),
        ..InitOptions::new(dir.path().to_path_buf())
    })
    .expect("a real address is no alias");
    assert_eq!(result.founder, REAL);
}
