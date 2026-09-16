// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The founding step from the command line on a machine with no git
//! identity (D3.9 of the forge connection NG design, JOY-0297-1A).
//!
//! These drive the real binary, because the question is what the CLI does
//! at its entry point: it decides the host kind once, and everything that
//! follows hangs off that decision.

use std::path::Path;
use std::process::Output;

/// Run `joy` in `root` on a machine with no git identity anywhere: its own
/// HOME, its own state and config directories, and git's own switch for
/// the system-wide config.
fn joy(root: &Path, home: &Path, args: &[&str]) -> Output {
    joy_process::command(env!("CARGO_BIN_EXE_joy"))
        .args(args)
        .current_dir(root)
        .env("HOME", home)
        .env("XDG_STATE_HOME", home.join(".state"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("JOY_SESSION")
        .env_remove("GIT_AUTHOR_EMAIL")
        .env_remove("GIT_COMMITTER_EMAIL")
        .env_remove("EMAIL")
        .output()
        .expect("joy runs")
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// A checkout directory and the person's home, both fresh.
fn machine() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    (dir, root, home)
}

/// The acceptance of J9: `joy init --user a@b.c` in a repository with no
/// git config succeeds, and the enrolment that follows enrols `a@b.c`
/// without reading git config.
#[test]
fn init_user_founds_and_auth_init_enrols_without_a_git_config() {
    let (_dir, root, home) = machine();

    let init = joy(
        &root,
        &home,
        &["init", "--name", "No Config", "--user", "a@b.c"],
    );
    assert!(init.status.success(), "{}", text(&init));
    let project = std::fs::read_to_string(root.join(".joy/project.yaml")).unwrap();
    assert!(project.contains("a@b.c"), "{project}");

    // The enrolment takes the member from the project, not from a git
    // config this machine does not have.
    let auth = joy(
        &root,
        &home,
        &[
            "auth",
            "init",
            "--passphrase",
            "correct horse battery staple",
        ],
    );
    assert!(auth.status.success(), "{}", text(&auth));
    assert!(
        text(&auth).contains("Authentication initialized for a@b.c"),
        "{}",
        text(&auth)
    );
    let project = std::fs::read_to_string(root.join(".joy/project.yaml")).unwrap();
    assert!(
        project.contains("verify-key") || project.contains("verify_key"),
        "{project}"
    );
}

/// D3.9: with no git config and nobody at the terminal (the test harness
/// is a pipe, not a terminal), `joy init` refuses with the named sentence
/// and leaves nothing behind.
#[test]
fn init_without_an_identity_and_without_a_person_refuses_by_name() {
    let (_dir, root, home) = machine();
    let init = joy(&root, &home, &["init", "--name", "Nobody"]);
    assert!(!init.status.success(), "{}", text(&init));
    assert!(
        text(&init)
            .contains("this project does not know who you are; run joy init --user <address>"),
        "{}",
        text(&init)
    );
    assert!(!root.join(".joy").exists());
}

/// The same command under a delegation session: refused with the same
/// sentence, and an agent is never asked to type an address.
#[test]
fn init_under_a_delegation_session_refuses_by_name() {
    let (_dir, root, home) = machine();
    let init = joy_process::command(env!("CARGO_BIN_EXE_joy"))
        .args(["init", "--name", "Delegated"])
        .current_dir(&root)
        .env("HOME", &home)
        .env("XDG_STATE_HOME", home.join(".state"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("JOY_SESSION", "sid:0000")
        .output()
        .expect("joy runs");
    assert!(!init.status.success(), "{}", text(&init));
    assert!(
        text(&init)
            .contains("this project does not know who you are; run joy init --user <address>"),
        "{}",
        text(&init)
    );
    assert!(!root.join(".joy").exists());
}
