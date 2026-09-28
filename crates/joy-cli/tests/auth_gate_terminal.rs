// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The auth gate on a terminal (operator decision 2026-09-27,
//! JOY-02B2-65): a person with no session who runs a command that
//! needs proof is ASKED for the passphrase, and the answer makes the
//! session, so the next command asks nothing. The other three states
//! (session, passphrase on the call, nothing and no terminal) are
//! driven in `auth_gate.rs`; this one needs a pty, which `openpty`
//! provides on unix only. The harness is `founder_terminal.rs`'s.

#![cfg(unix)]

use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::Path;
use std::process::Stdio;

const PASSPHRASE: &str = "correct horse battery staple";

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
    let mut command = joy_process::command(env!("CARGO_BIN_EXE_joy"));
    command
        .args(args)
        .current_dir(root)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_STATE_HOME", home.join(".state"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("JOY_SESSION")
        .env_remove("JOY_PASSPHRASE")
        .env_remove("JOY_PASSPHRASE_STDIN")
        .env_remove("JOY_USER");
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

fn machine() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    (dir, root, home)
}

/// `joy add` in a project with an AI member, signed out, on a terminal:
/// the passphrase is asked, the item is created, the session is made,
/// and the next command asks nothing.
#[test]
fn a_guarded_write_on_a_terminal_asks_once_and_signs_in() {
    let (_dir, root, home) = machine();
    std::fs::write(
        home.join(".gitconfig"),
        "[user]\n\temail = a@b.c\n\tname = Somebody\n",
    )
    .unwrap();
    let (ok, seen) = joy(
        &root,
        &home,
        &["init", "--name", "Ledger", "--acronym", "LG"],
    );
    assert!(ok, "{seen}");
    let (ok, seen) = joy(&root, &home, &["auth", "init", "--passphrase", PASSPHRASE]);
    assert!(ok, "{seen}");
    let (ok, seen) = joy(
        &root,
        &home,
        &[
            "project",
            "member",
            "add",
            "ai:claude@joy",
            "--capabilities",
            "implement",
            "--passphrase",
            PASSPHRASE,
        ],
    );
    assert!(ok, "{seen}");
    // Another machine: the project travels, the session does not.
    std::fs::remove_dir_all(home.join(".state")).unwrap();

    let (ok, seen) = joy_on_a_terminal(
        &root,
        &home,
        &["add", "task", "Asked for"],
        &format!("{PASSPHRASE}\n"),
    );
    assert!(ok, "{seen}");
    assert!(
        seen.contains("Passphrase:"),
        "the passphrase was asked: {seen}"
    );
    assert!(
        seen.contains("Authenticated as a@b.c. Session active (24h)."),
        "and the session was made: {seen}"
    );
    assert!(seen.contains("LG-0001"), "the item was created: {seen}");

    // The session is on disk, bound to that terminal (human sessions
    // are, so a command run without one below would not see it): the
    // file names the member, the project and the terminal device.
    let sessions = home.join(".state").join("joy").join("sessions");
    let files: Vec<_> = std::fs::read_dir(&sessions)
        .expect("the session directory exists")
        .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
        .collect();
    assert_eq!(files.len(), 1, "one session: {files:?}");
    assert!(files[0].contains("\"member\": \"a@b.c\""), "{}", files[0]);
    assert!(files[0].contains("\"project_id\": \"LG\""), "{}", files[0]);
    assert!(files[0].contains("\"tty\": \"/dev/"), "{}", files[0]);
}
