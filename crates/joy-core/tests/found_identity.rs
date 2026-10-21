// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! `found_identity` (JOY-02E0-EC): what this machine offers as an
//! identity for a project, read through the resolver's own git config
//! read and connector call.
//!
//! ONE test in its own binary, for the reason resolve_identity_order.rs
//! gives: HOME, libgit2's config search paths and the plugin search
//! directories are process state. Unix only, because the stub connector
//! is a shell script.

#![cfg(unix)]

joy_test_env::isolate!();

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use joy_core::forge_plugins;
use joy_core::identity::{found_identity, FoundForge, FoundIdentity};
use joy_core::init::{init, InitOptions};

/// A machine with no git identity anywhere (resolve_identity_order.rs's
/// helper of the same name and purpose).
fn a_machine_without_a_git_identity() -> tempfile::TempDir {
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
    home
}

fn git_config_says_globally(home: &Path, email: &str) {
    std::fs::write(
        home.join(".gitconfig"),
        format!("[user]\n\temail = {email}\n\tname = Somebody\n"),
    )
    .unwrap();
}

/// Write the stub connector: `identity_json` is its raw `identity` answer.
fn connector(dir: &Path, identity_json: &str) {
    let body = r#"#!/bin/sh
if [ "$1" = version ]; then
  echo '{"protocol":2,"plugin":"test stub","forges":["github","gitlab","gitea"]}'
  exit 0
fi
shift
verb="$1"
case "$verb" in
  claims) echo '{"claims":true}' ;;
  identity) echo '__IDENTITY__' ;;
  *) echo '{}' ;;
esac
"#
    .replace("__IDENTITY__", identity_json);
    let path = dir.join(forge_plugins::COMBINED_BINARY);
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(body.as_bytes()).unwrap();
    file.flush().unwrap();
    drop(file);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn github(login: Option<&str>, tool: Option<&str>) -> Option<FoundForge> {
    Some(FoundForge {
        name: "GitHub".into(),
        login: login.map(str::to_string),
        tool: tool.map(str::to_string),
    })
}

#[test]
fn what_this_machine_offers_with_and_without_git_and_forge() {
    let home = a_machine_without_a_git_identity();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init(InitOptions {
        name: Some("Found".into()),
        acronym: Some("FO".into()),
        user: Some("a@b.c".to_string()),
        ..InitOptions::new(root.to_path_buf())
    })
    .unwrap();
    let project = joy_core::store::load_project(root).unwrap();
    std::env::remove_var(forge_plugins::PLUGIN_DIR_ENV);
    let plugins = tempfile::tempdir().unwrap();
    forge_plugins::set_plugin_dirs(vec![plugins.path().to_path_buf()]);

    // No git config and no remote: nothing is offered.
    assert_eq!(found_identity(root, &project), FoundIdentity::default());

    // Git only: the address, and no forge without a remote.
    git_config_says_globally(home.path(), "eve@example.com");
    assert_eq!(
        found_identity(root, &project),
        FoundIdentity {
            addresses: vec!["eve@example.com".into()],
            forge: None,
        }
    );

    // A forge account with addresses: git first, then the forge's own,
    // the duplicate only once; the connector names its tool.
    git2::Repository::open(root)
        .unwrap()
        .remote("origin", "git@github.example.com:o/r.git")
        .unwrap();
    connector(
        plugins.path(),
        r#"{"known":true,"login":"eve","user_id":"7","emails":["Eve@example.com","eve@work.example"],"cli":"gh"}"#,
    );
    assert_eq!(
        found_identity(root, &project),
        FoundIdentity {
            addresses: vec!["eve@example.com".into(), "eve@work.example".into()],
            forge: github(Some("eve"), Some("gh")),
        }
    );

    // A forge account that shows no readable address: the login alone.
    std::fs::remove_file(home.path().join(".gitconfig")).unwrap();
    connector(
        plugins.path(),
        r#"{"known":true,"login":"eve","user_id":"7","emails":[],"cli":"gh"}"#,
    );
    assert_eq!(
        found_identity(root, &project),
        FoundIdentity {
            addresses: vec![],
            forge: github(Some("eve"), Some("gh")),
        }
    );

    // Nobody signed in there: the forge and its tool, no login.
    connector(plugins.path(), r#"{"known":false,"cli":"gh"}"#);
    assert_eq!(
        found_identity(root, &project),
        FoundIdentity {
            addresses: vec![],
            forge: github(None, Some("gh")),
        }
    );
}
