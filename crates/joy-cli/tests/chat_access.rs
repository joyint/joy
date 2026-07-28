// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! Who can read a sealed chat from the command line, and after what.
//!
//! These drive the real binary against a real project, because that is
//! where the answer actually lives: the seed a session holds decides what
//! opens, and every hop in between (project.yaml, the state dir, the
//! delegation token) is part of the answer.

use std::path::Path;
use std::process::Command;

const PASS: &str = "correct horse battery staple";

/// Run `joy` in `root` with its own HOME, and return stdout.
fn joy(root: &Path, home: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_joy"))
        .args(args)
        .current_dir(root)
        .env("HOME", home)
        .env("XDG_STATE_HOME", home.join(".state"))
        .output()
        .expect("joy runs");
    assert!(
        out.status.success(),
        "joy {args:?} failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// A project with one enrolled member, set up the way a person does it.
fn project(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = dir.join("project");
    let home = dir.join("home");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git runs");
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "member@joyint.test"]);
    git(&["config", "user.name", "Member"]);
    joy(
        &root,
        &home,
        &["init", "--name", "Access", "--acronym", "AC"],
    );
    joy(&root, &home, &["auth", "init", "--passphrase", PASS]);
    (root, home)
}

#[test]
fn a_passphrase_change_keeps_the_chats_readable() {
    // The wrapped-seed model (ADR-039) keeps the identity seed across a
    // passphrase change, so the chat slots stay valid and nothing has to
    // be re-wrapped. That is worth a test rather than a memory: the
    // opposite would silently cost a person their whole history, and the
    // command's own help text still claims it re-derives the keypair.
    let dir = tempfile::tempdir().unwrap();
    let (root, home) = project(dir.path());
    joy(
        &root,
        &home,
        &[
            "chat",
            "send",
            "general",
            "vor dem wechsel",
            "--passphrase",
            PASS,
        ],
    );

    joy(
        &root,
        &home,
        &[
            "auth",
            "--passphrase",
            PASS,
            "passphrase",
            "--new-passphrase",
            "ein ganz anderer satz",
        ],
    );

    let shown = joy(
        &root,
        &home,
        &[
            "chat",
            "show",
            "general",
            "--passphrase",
            "ein ganz anderer satz",
        ],
    );
    assert!(
        shown.contains("vor dem wechsel"),
        "the history must survive a passphrase change: {shown}"
    );
}

/// Run `joy`, returning stdout even when the command failed (the caller
/// asserts on the content).
fn joy_try(root: &Path, home: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_joy"))
        .args(args)
        .current_dir(root)
        .env("HOME", home)
        .env("XDG_STATE_HOME", home.join(".state"))
        .output()
        .expect("joy runs");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn an_ai_reads_the_chat_with_its_own_session() {
    // An AI never has a passphrase: it acts with a token, and the
    // session that token redeems carries its key. Before this, `joy chat
    // show --session ...` answered "no chat with id general" in a room
    // the AI is a member of, and only the platform could read, through a
    // path of its own (JOY-023E-68).
    let dir = tempfile::tempdir().unwrap();
    let (root, home) = project(dir.path());

    joy(
        &root,
        &home,
        &[
            "project",
            "member",
            "add",
            "ai:vibe@joy",
            "--with-token",
            "--passphrase",
            PASS,
        ],
    );
    // The crypt scope is what puts the delegation key into the token, and
    // that key is the AI's identity: without it there is nothing to read
    // the chat with, which is a grant the operator makes on purpose.
    let issued = joy(
        &root,
        &home,
        &[
            "auth",
            "--passphrase",
            PASS,
            "token",
            "add",
            "ai:vibe@joy",
            "--crypt",
        ],
    );
    let token = issued
        .split(|c: char| c.is_whitespace() || c == '"')
        .find(|w| w.starts_with("joy_t_"))
        .expect("the add prints the delegation token")
        .to_string();

    joy(
        &root,
        &home,
        &[
            "chat",
            "send",
            "general",
            "hallo an alle",
            "--passphrase",
            PASS,
        ],
    );

    let redeemed = joy(&root, &home, &["auth", "--token", &token, "--json"]);
    let session = redeemed
        .split(|c: char| c.is_whitespace() || c == '"')
        .find(|w| w.starts_with("joy_s_"))
        .expect("redeeming prints the session")
        .to_string();

    let shown = joy_try(
        &root,
        &home,
        &["chat", "show", "general", "--session", &session],
    );
    assert!(
        shown.contains("hallo an alle"),
        "an AI member must read the chat with its session: {shown}"
    );

    // …and no key means no content, for anyone.
    let empty = joy_try(&root, &home, &["chat", "show", "general"]);
    assert!(
        !empty.contains("hallo an alle"),
        "no key, no content: {empty}"
    );
}

#[test]
fn an_ai_without_a_chat_key_is_told_why() {
    // Auth-only delegation: the AI can act, but it holds no key, so the
    // chats stay closed. "No chats" would read as an empty room; the
    // room is there and the key is not (JOY-023E-68).
    let dir = tempfile::tempdir().unwrap();
    let (root, home) = project(dir.path());
    joy(
        &root,
        &home,
        &[
            "project",
            "member",
            "add",
            "ai:vibe@joy",
            "--with-token",
            "--passphrase",
            PASS,
        ],
    );
    // act as the AI, with no session at all
    Command::new("git")
        .args(["config", "user.email", "ai:vibe@joy"])
        .current_dir(&root)
        .output()
        .expect("git runs");

    let said = joy_try(&root, &home, &["chat", "ls"]);
    assert!(
        said.contains("no chat key"),
        "the AI must hear WHY it sees nothing: {said}"
    );
}
