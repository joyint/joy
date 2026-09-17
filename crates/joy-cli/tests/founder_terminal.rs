// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The founding question with a real terminal on the other end (D3.9 and
//! D1.1 of the forge connection NG design, JOY-0297-1A).
//!
//! Two of J9's acceptance criteria are about what `joy init` does for a
//! PERSON at a terminal, and neither can be proven by a test harness that
//! hands joy a pipe: with a pipe the host is `Background` whatever else
//! is true, so the interactive branch is never entered and a refusal
//! under `JOY_SESSION` proves nothing about the delegated one. So these
//! cases give joy a pty and type into it.
//!
//! Unix only, because `openpty` is. The rule they prove is platform
//! independent; on Windows the host decision is the same code with the
//! same input (`joy_core::host::HostKind::detect`), covered by
//! joy-core's own cases.
#![cfg(unix)]

use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::Path;
use std::process::Stdio;

/// A pty pair: what the test holds, and what the child gets as its
/// stdin, stdout and stderr.
struct Terminal {
    person: std::fs::File,
    child: OwnedFd,
}

impl Terminal {
    fn open() -> Terminal {
        let mut controller = 0;
        let mut follower = 0;
        // SAFETY: openpty writes two valid descriptors or returns -1, and
        // both are taken over by owning types below.
        let rc = unsafe {
            libc::openpty(
                &mut controller,
                &mut follower,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(rc, 0, "openpty");
        // SAFETY: both descriptors come from openpty and are owned here.
        unsafe {
            Terminal {
                person: std::fs::File::from_raw_fd(controller),
                child: OwnedFd::from_raw_fd(follower),
            }
        }
    }

    /// The child's three standard streams, all on the terminal, which is
    /// what makes `joy` call this host `Interactive`.
    fn stdio(&self) -> [Stdio; 3] {
        [
            Stdio::from(self.child.try_clone().unwrap()),
            Stdio::from(self.child.try_clone().unwrap()),
            Stdio::from(self.child.try_clone().unwrap()),
        ]
    }
}

/// Run `joy` on a terminal with `answers` typed into it, and return
/// everything the person would have seen plus the exit status.
fn joy_on_a_terminal(
    root: &Path,
    home: &Path,
    args: &[&str],
    session: Option<&str>,
    answers: &str,
) -> (bool, String) {
    let terminal = Terminal::open();
    let [stdin, stdout, stderr] = terminal.stdio();
    let mut command = joy_process::command(env!("CARGO_BIN_EXE_joy"));
    command
        .args(args)
        .current_dir(root)
        .env("HOME", home)
        .env("XDG_STATE_HOME", home.join(".state"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(stdin)
        .stdout(stdout)
        .stderr(stderr);
    match session {
        Some(value) => command.env("JOY_SESSION", value),
        None => command.env_remove("JOY_SESSION"),
    };
    let mut child = command.spawn().expect("joy runs");
    // The child holds its own copies now, and every copy on this side has
    // to go: the three the `Command` still owns and the one the pair was
    // opened with. While any of them is open, reading the terminal would
    // never see the end of the child's output.
    drop(command);
    drop(terminal.child);

    let mut person = terminal.person;
    person.write_all(answers.as_bytes()).unwrap();
    person.flush().unwrap();

    // Drain the terminal while the child runs, so a long answer cannot
    // fill the buffer and deadlock the two of us.
    let mut reader = person.try_clone().unwrap();
    let drain = std::thread::spawn(move || {
        let mut seen = Vec::new();
        let mut buffer = [0u8; 4096];
        while let Ok(read) = reader.read(&mut buffer) {
            if read == 0 {
                break;
            }
            seen.extend_from_slice(&buffer[..read]);
        }
        String::from_utf8_lossy(&seen).to_string()
    });

    let status = wait_with_a_bound(&mut child);
    drop(person);
    let seen = drain.join().unwrap_or_default();
    (status, seen)
}

/// Wait for the child, and kill it rather than hang the suite if it is
/// waiting for an answer the case did not script.
fn wait_with_a_bound(child: &mut std::process::Child) -> bool {
    for _ in 0..600 {
        match child.try_wait().unwrap() {
            Some(status) => return status.success(),
            None => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
    let _ = child.kill();
    panic!("joy did not finish: it is waiting for an answer nobody scripted");
}

fn machine() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    (dir, root, home)
}

/// The acceptance of J9: `joy init` with no git config ON A TERMINAL asks
/// for the address and completes. The first answer is the anonymous mode
/// question `joy init` asks a person, the second is the address.
#[test]
fn init_on_a_terminal_asks_for_the_address_and_completes() {
    let (_dir, root, home) = machine();

    let (ok, seen) = joy_on_a_terminal(
        &root,
        &home,
        &["init", "--name", "Asked"],
        None,
        "n\ntyped@example.com\n",
    );

    assert!(ok, "{seen}");
    assert!(
        seen.contains("git config names nobody"),
        "the person was asked: {seen}"
    );
    assert!(seen.contains("Address:"), "the person was asked: {seen}");
    let project = std::fs::read_to_string(root.join(".joy/project.yaml")).unwrap();
    assert!(
        project.contains("typed@example.com"),
        "the typed address founded the project: {project}"
    );
}

/// The acceptance of J9 for the delegated host: THE SAME COMMAND, the
/// same terminal, the same missing git config, under a live `JOY_SESSION`
/// refuses with the named sentence and asks nobody. An agent that owns a
/// terminal is still an agent (D1.1), so the terminal is what makes this
/// case worth anything: without one, `Background` would have refused for
/// its own reason and the session would prove nothing.
#[test]
fn init_on_a_terminal_under_a_delegation_refuses_by_name() {
    let (_dir, root, home) = machine();
    let session = a_live_session(&home);

    let (ok, seen) = joy_on_a_terminal(
        &root,
        &home,
        &["init", "--name", "Delegated"],
        Some(&session),
        // Enough answers that a joy which DID ask would have completed:
        // the case has to fail on the refusal, not on an empty stdin.
        "n\ntyped@example.com\n",
    );

    assert!(!ok, "{seen}");
    assert!(
        seen.contains("this project does not know who you are; run joy init --user <address>"),
        "{seen}"
    );
    assert!(
        !seen.contains("Address:"),
        "a delegated host asks nobody: {seen}"
    );
    assert!(!root.join(".joy").exists());
}

/// A `JOY_SESSION` that names a session joy can really load, minted the
/// way an agent gets one: a project of its own with an AI member, a
/// delegation token, and the redemption that prints the handle.
///
/// It lives in the same `home` as the case under test, because that is
/// where joy looks for the session file.
fn a_live_session(home: &Path) -> String {
    let root = home.join("delegator");
    std::fs::create_dir_all(&root).unwrap();
    let joy = |args: &[&str]| -> (bool, String) {
        let out = joy_process::command(env!("CARGO_BIN_EXE_joy"))
            .args(args)
            .current_dir(&root)
            .env("HOME", home)
            .env("XDG_STATE_HOME", home.join(".state"))
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("JOY_SESSION")
            .output()
            .expect("joy runs");
        (
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    };
    let passphrase = "correct horse battery staple";
    let (ok, seen) = joy(&["init", "--name", "Delegator", "--user", "human@example.com"]);
    assert!(ok, "{seen}");
    // This checkout gets a git identity of its own: the commands that
    // mint a delegation are J11's call sites and still read one. The
    // checkout under test never sees it (its own HOME is empty and this
    // config is repository local), which is the whole point of the case.
    let repo = git2::Repository::open(&root).unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "The Human").unwrap();
    config.set_str("user.email", "human@example.com").unwrap();
    for args in [
        vec!["auth", "init", "--passphrase", passphrase],
        vec![
            "project",
            "member",
            "add",
            "ai:claude@joy",
            "--passphrase",
            passphrase,
        ],
    ] {
        let (ok, seen) = joy(&args);
        assert!(ok, "{args:?}: {seen}");
    }
    let (ok, token) = joy(&[
        "auth",
        "token",
        "add",
        "ai:claude@joy",
        "--passphrase",
        passphrase,
    ]);
    assert!(ok, "{token}");
    // The token is the quoted line among the instructions.
    let token = token
        .lines()
        .find_map(|line| line.trim().strip_prefix('"')?.strip_suffix('"'))
        .expect("the token is printed")
        .to_string();
    let (ok, handle) = joy(&["auth", "--token", &token]);
    assert!(ok, "{handle}");
    handle
        .lines()
        .find_map(|line| line.strip_prefix("export JOY_SESSION="))
        .expect("the redemption prints the handle")
        .trim()
        .to_string()
}
