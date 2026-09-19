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
///
/// `USERPROFILE`, `HOMEDRIVE` and `HOMEPATH` move with `HOME`, because
/// on Windows libgit2 looks for the global config in EVERY one of those
/// that exists (`git_win32__find_global_dirs`), so moving `HOME` alone
/// still found the machine's own `.gitconfig` - and the windows-latest
/// CI job writes an identity into it on purpose.
fn joy(root: &Path, home: &Path, args: &[&str]) -> Output {
    joy_process::command(env!("CARGO_BIN_EXE_joy"))
        .args(args)
        .current_dir(root)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("HOMEDRIVE", "")
        .env("HOMEPATH", "")
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
/// without reading git config, given the same `--user` explicitly. A
/// later addition to the operator's 2026-09-19 correction (JOY-02AE-1A)
/// retired the device pin from `acting_member` too, so `auth init` no
/// longer finds the founder through it either: naming `--user` at both
/// steps is what a founder with no git config actually has to do now,
/// and what happens AFTER enrolment needs the git config back, since
/// resolve_identity reads it, not a pin.
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

    // The enrolment takes the member from `--user`, not from a git
    // config this machine does not have, and not from a pin either.
    let auth = joy(
        &root,
        &home,
        &[
            "auth",
            "init",
            "--user",
            "a@b.c",
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

    // Operator decision 2026-09-19 (JOY-02AE-1A, correcting D3.9): the
    // device pin `auth init` used to leave behind is not read for
    // identity any more, so the next command needs the git config back
    // to know who acts here, exactly as a working checkout would have
    // one.
    std::fs::write(
        home.join(".gitconfig"),
        "[user]\n\temail = a@b.c\n\tname = Founder\n",
    )
    .unwrap();
    let add = joy(&root, &home, &["add", "task", "First thing"]);
    assert!(add.status.success(), "{}", text(&add));

    // The session lasts 24 hours; the project does not end with it.
    // Re-authenticating finds the same member the same way, so the
    // founder is not locked out of his own project tomorrow.
    let again = joy(
        &root,
        &home,
        &["auth", "--passphrase", "correct horse battery staple"],
    );
    assert!(again.status.success(), "{}", text(&again));
    assert!(text(&again).contains("a@b.c"), "{}", text(&again));
    let status = joy(&root, &home, &["auth", "status"]);
    assert!(
        text(&status).contains("a@b.c"),
        "the session names the founder: {}",
        text(&status)
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

/// A `JOY_SESSION` that names no live session is no delegation: the host
/// is what it was without it, here a background one, and it refuses with
/// the same sentence rather than tripping over the value.
///
/// This case proves exactly that and no more: with a pipe for a terminal
/// the refusal would have come anyway. The acceptance criterion about a
/// delegated host ("the same command under JOY_SESSION refuses with the
/// named sentence") needs a terminal to mean anything, and it is covered
/// in `founder_terminal.rs`, where joy gets one and a real session.
#[test]
fn init_with_a_stale_session_value_refuses_like_a_background_host() {
    let (_dir, root, home) = machine();
    let init = joy_process::command(env!("CARGO_BIN_EXE_joy"))
        .args(["init", "--name", "Delegated"])
        .current_dir(&root)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("HOMEDRIVE", "")
        .env("HOMEPATH", "")
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

/// The acceptance of J9 in the product: a commit joy writes in an
/// anonymous mode project carries the opaque `m-<hex>` id in BOTH
/// signature fields. The git config of this checkout names a person by
/// name and address, and none of it may reach the commit (ADR-042).
#[test]
fn an_anonymous_project_commits_under_the_opaque_id() {
    let (_dir, root, home) = machine();

    let init = joy(
        &root,
        &home,
        &[
            "init",
            "--name",
            "Anon",
            "--user",
            "scotty@example.com",
            "--anonymous",
            "--passphrase",
            "correct horse battery staple",
        ],
    );
    assert!(init.status.success(), "{}", text(&init));

    // This machine DOES have a git identity, in the checkout itself, and
    // it is the identity `git commit` would have signed with.
    let repo = git2::Repository::open(&root).unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "Scotty Real").unwrap();
    config.set_str("user.email", "scotty@example.com").unwrap();

    // joy commits its own writes from here on.
    std::fs::write(
        root.join(".joy/config.yaml"),
        "workflow:\n  auto-git: commit\n",
    )
    .unwrap();

    let add = joy(&root, &home, &["add", "task", "First thing"]);
    assert!(add.status.success(), "{}", text(&add));

    let head = repo.head().unwrap().peel_to_commit().unwrap();
    assert!(
        head.summary()
            .ok()
            .flatten()
            .unwrap_or("")
            .starts_with("joy: add"),
        "joy wrote the commit at HEAD: {:?}",
        head.summary().ok().flatten()
    );
    let fields = [
        head.author().name().unwrap().to_string(),
        head.author().email().unwrap().to_string(),
        head.committer().name().unwrap().to_string(),
        head.committer().email().unwrap().to_string(),
    ];
    for field in &fields {
        assert!(
            field.starts_with("m-"),
            "every signature field is the opaque member id, got {fields:?}"
        );
        assert!(!field.contains('@'), "no address in {fields:?}");
        assert!(!field.contains("Scotty"), "no person's name in {fields:?}");
    }
    assert_eq!(fields[0], fields[1]);
    assert_eq!(fields[0], fields[2]);
    assert_eq!(fields[0], fields[3]);
}
