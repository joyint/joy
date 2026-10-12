// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The strength line above the prompt when a passphrase is CHOSEN on a
//! terminal (JI-01A5-93, JOY-02CC-46): `joy auth init` draws it, it
//! changes with the typing, a weak passphrase is accepted all the same,
//! and nothing typed is echoed. The harness is `founder_terminal.rs`'s;
//! `openpty` is unix only.

#![cfg(unix)]

joy_test_env::isolate!();

use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::Path;
use std::process::Stdio;

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
        // The answers are typed before joy asks, and a terminal echoes
        // what is typed; with echo off, anything of the passphrase in
        // the output is joy's doing.
        // SAFETY: termios is read and written on the descriptor openpty
        // just returned.
        unsafe {
            let mut termios: libc::termios = std::mem::zeroed();
            assert_eq!(libc::tcgetattr(follower, &mut termios), 0, "tcgetattr");
            termios.c_lflag &= !libc::ECHO;
            assert_eq!(
                libc::tcsetattr(follower, libc::TCSANOW, &termios),
                0,
                "tcsetattr"
            );
        }
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

/// Run `joy` on a terminal with `answers` typed into it, and return
/// everything the person would have seen plus the exit status.
fn joy_on_a_terminal(root: &Path, home: &Path, args: &[&str], answers: &str) -> (bool, String) {
    let terminal = Terminal::open();
    let [stdin, stdout, stderr] = terminal.stdio();
    let mut command = joy_test_env::command(env!("CARGO_BIN_EXE_joy"));
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
    // The confirm prompt reads `/dev/tty`, not stdin, so the pty has to
    // be the child's CONTROLLING terminal: a new session, and the
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
    (status, plain(&seen))
}

/// The text without its colour codes: the terminal is a tty, so the bar
/// is coloured, and the cases read the words.
fn plain(seen: &str) -> String {
    let mut out = String::new();
    let mut rest = seen;
    while let Some(at) = rest.find("\x1b[") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 2..];
        match after.find(|c: char| c.is_ascii_alphabetic()) {
            Some(end) => rest = &after[end + 1..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn wait_with_a_bound(child: &mut std::process::Child) -> bool {
    for _ in 0..600 {
        match child.try_wait().unwrap() {
            Some(status) => return status.success(),
            None => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
    // Kill it rather than hang the suite; the case then shows what the
    // person saw up to there.
    let _ = child.kill();
    false
}

/// A project whose founder is named by the repository's git config, so
/// `joy auth init` knows who enrols and asks for the passphrase alone.
fn project() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let git = |args: &[&str]| {
        let ok = joy_test_env::command("git")
            .args(args)
            .current_dir(&root)
            .env("HOME", &home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "founder@example.com"]);
    git(&["config", "user.name", "Founder"]);
    let init = joy_test_env::command(env!("CARGO_BIN_EXE_joy"))
        .args(["init", "--name", "Meter"])
        .current_dir(&root)
        .env("HOME", &home)
        .env("XDG_STATE_HOME", home.join(".state"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "joy init: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    (dir, root, home)
}

/// Three words of one letter are accepted, and the person saw the line
/// say so in red while typing: no minimum, a warning instead.
#[test]
fn a_weak_passphrase_is_accepted_and_the_line_said_insufficient() {
    let (_dir, root, home) = project();

    let (ok, seen) = joy_on_a_terminal(&root, &home, &["auth", "init"], "q w e\rq w e\r");

    assert!(ok, "{seen}");
    assert!(seen.contains("Choose a passphrase:"), "{seen}");
    assert!(
        seen.contains("□□□□□ insufficient (entropy 0 bit)"),
        "the line stood before the first key: {seen}"
    );
    assert!(
        seen.contains("■□□□□ insufficient (entropy 17 bit)"),
        "the line followed the typing: {seen}"
    );
    assert!(seen.contains("RECOVERY KEY"), "{seen}");
    assert!(!seen.contains("q w e"), "nothing typed is echoed: {seen}");
}

/// Six random words fill the bar and the line says excellent.
#[test]
fn a_strong_passphrase_fills_the_bar() {
    let (_dir, root, home) = project();
    let passphrase = "ladle quartz nimbus fjord carapace velvet";

    let (ok, seen) = joy_on_a_terminal(
        &root,
        &home,
        &["auth", "init"],
        &format!("{passphrase}\r{passphrase}\r"),
    );

    assert!(ok, "{seen}");
    assert!(seen.contains("■■■■■ excellent (entropy"), "{seen}");
    assert!(
        !seen.contains(passphrase),
        "nothing typed is echoed: {seen}"
    );
}
