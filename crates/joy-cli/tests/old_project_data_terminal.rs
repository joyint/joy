// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! A project whose data is from an older joy, at a terminal: the
//! question at the bottom, `[Y/n]`, and what a yes and a no do, for
//! `joy update` and for the one command that needs the update (`joy
//! project member edit <ai>` of a person for themselves). The fixture is
//! `tests/fixtures/members/open`, a project in joy 0.22's format. The
//! harness is `auth_gate_terminal.rs`'s.

#![cfg(unix)]

joy_test_env::isolate!();

use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::Stdio;

const FOUNDER: &str = "founder@example.com";
const FOUNDER_PASS: &str = "correct horse battery staple extra words";
const QUESTION: &str =
    "Project data is from an older joy. Update? Everyone then needs joy 0.24+ [Y/n]";

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

    fn stdio(&self) -> [Stdio; 3] {
        [
            Stdio::from(self.child.try_clone().unwrap()),
            Stdio::from(self.child.try_clone().unwrap()),
            Stdio::from(self.child.try_clone().unwrap()),
        ]
    }
}

fn base_command(root: &Path, home: &Path, args: &[&str]) -> std::process::Command {
    let mut command = joy_test_env::command(env!("CARGO_BIN_EXE_joy"));
    command
        .args(args)
        .current_dir(root)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_STATE_HOME", home.join(".state"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_NOSYSTEM", "1");
    command
}

/// Run `joy` with no terminal, output captured.
fn joy(root: &Path, home: &Path, args: &[&str]) -> (bool, String) {
    let output = base_command(root, home, args).output().expect("joy runs");
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

/// Run `joy` on a terminal with `answers` typed into it, and return
/// everything the person would have seen plus the exit status.
fn joy_on_a_terminal(root: &Path, home: &Path, args: &[&str], answers: &str) -> (bool, String) {
    let terminal = Terminal::open();
    let [stdin, stdout, stderr] = terminal.stdio();
    let mut command = base_command(root, home, args);
    command.stdin(stdin).stdout(stdout).stderr(stderr);
    // The passphrase prompt reads `/dev/tty`, not stdin, so the pty has
    // to be the child's CONTROLLING terminal: a new session, and the
    // terminal on its stdin claimed as that session's own.
    // SAFETY: both calls are async-signal-safe and only touch the child.
    unsafe {
        use std::os::unix::process::CommandExt;
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().expect("joy runs");
    drop(command);
    drop(terminal.child);

    let mut person = terminal.person;
    person.write_all(answers.as_bytes()).unwrap();
    person.flush().unwrap();

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

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A fresh clone of the open 0.22 fixture on a machine that knows nobody.
fn project_from_before() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/members/open");
    copy_tree(&fixture.join("dot-joy"), &root.join(".joy"));
    git2::Repository::init(&root).unwrap();
    (dir, root, home)
}

fn member_files(root: &Path) -> usize {
    std::fs::read_dir(root.join(".joy/members"))
        .map(|dir| dir.count())
        .unwrap_or(0)
}

/// `joy update` asks at the bottom; a yes and the passphrase update the
/// project in the same run.
#[test]
fn joy_update_asks_and_a_yes_updates_the_project() {
    let (_dir, root, home) = project_from_before();
    let (ok, seen) = joy_on_a_terminal(
        &root,
        &home,
        &["update", "--no-binary", "--force", "--user", FOUNDER],
        &format!("y\n{FOUNDER_PASS}\n"),
    );
    assert!(ok, "{seen}");
    assert!(seen.contains(QUESTION), "{seen}");
    assert!(seen.contains("updated"), "{seen}");
    assert_eq!(member_files(&root), 3, "{seen}");
}

/// A no leaves the project as it is and says what that means.
#[test]
fn joy_update_after_a_no_leaves_the_project_as_it_is() {
    let (_dir, root, home) = project_from_before();
    let (ok, seen) = joy_on_a_terminal(
        &root,
        &home,
        &["update", "--no-binary", "--force", "--user", FOUNDER],
        "n\n",
    );
    assert!(ok, "{seen}");
    assert!(seen.contains(QUESTION), "{seen}");
    assert!(
        seen.contains("from an older joy, some features don't work"),
        "{seen}"
    );
    assert!(
        !seen.contains("Passphrase"),
        "a no asks nothing more: {seen}"
    );
    assert_eq!(member_files(&root), 0);
}

/// Your own model for an AI member needs the update: the command asks,
/// and after a yes it goes on and sets the model.
#[test]
fn your_own_model_asks_for_the_update_and_then_is_set() {
    let (_dir, root, home) = project_from_before();
    let (ok, seen) = joy_on_a_terminal(
        &root,
        &home,
        &[
            "project", "member", "edit", "claude", "--model", "opus", "--user", FOUNDER,
        ],
        &format!("y\n{FOUNDER_PASS}\n"),
    );
    assert!(ok, "{seen}");
    assert!(seen.contains(QUESTION), "{seen}");
    assert!(seen.contains("claude runs on opus for you."), "{seen}");
    assert_eq!(member_files(&root), 3, "{seen}");
}

/// Without a terminal nobody is asked: the command says what needs the
/// update, in plain words, and changes nothing.
#[test]
fn without_a_terminal_your_own_model_says_it_needs_the_update() {
    let (_dir, root, home) = project_from_before();
    let (ok, seen) = joy(
        &root,
        &home,
        &[
            "project",
            "member",
            "edit",
            "claude",
            "--model",
            "opus",
            "--user",
            FOUNDER,
            "--passphrase",
            FOUNDER_PASS,
        ],
    );
    assert!(!ok, "{seen}");
    assert!(
        seen.contains(
            "Your own model for claude needs an update of this project. \
             Run joy update at a terminal."
        ),
        "{seen}"
    );
    assert!(!seen.contains("member files"), "{seen}");
    assert_eq!(member_files(&root), 0);
}
