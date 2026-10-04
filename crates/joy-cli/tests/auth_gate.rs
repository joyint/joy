// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! One authentication gate for every command (operator decision
//! 2026-09-27, JOY-02B2-65), driven through the real binary.
//!
//! Every command that needs proof behaves the same way on the four
//! states a person can be in: (1) a live session, nothing is asked;
//! (2) no session and the passphrase on the call (the global
//! `--passphrase`), the command runs and the session is made; (3) no
//! session and a terminal, the passphrase is asked and the session is
//! made; (4) no session, no passphrase, no terminal, the refusal names
//! both ways. Under `--user` the passphrase proves the one call and no
//! session is made. One command of each group is driven: the guard
//! group (`joy add` in a project with an AI member), the crypt group
//! (`joy crypt add`, and `joy ls` reading it back), and the member
//! group (`joy project member add`).

// Nothing of the developer's shell and session reaches this binary
// (JOY-02BB-C7).
joy_test_env::isolate!();

use std::path::PathBuf;
use std::process::Output;

const PASSPHRASE: &str = "correct horse battery staple";

/// A machine: a checkout, a home with its own state and config
/// directories, and no git identity from anywhere else (copied from
/// `identity_call_sites.rs`'s helper of the same name and purpose).
struct Machine {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
}

impl Machine {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        Machine {
            _dir: dir,
            root,
            home,
        }
    }

    fn command(&self, args: &[&str]) -> std::process::Command {
        let mut command = joy_test_env::command(env!("CARGO_BIN_EXE_joy"));
        command
            .args(args)
            .current_dir(&self.root)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("HOMEDRIVE", "")
            .env("HOMEPATH", "")
            .env("XDG_STATE_HOME", self.home.join(".state"))
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1");
        command
    }

    /// Run `joy` with no terminal: stdin closed, output captured.
    fn joy(&self, args: &[&str]) -> Output {
        self.command(args).output().expect("joy runs")
    }

    /// Write a global `user.email`, the way `git config --global` would.
    fn git_config_says(&self, email: &str) {
        std::fs::write(
            self.home.join(".gitconfig"),
            format!("[user]\n\temail = {email}\n\tname = Somebody\n"),
        )
        .unwrap();
    }

    /// Model another machine, or a fresh clone: the project file travels,
    /// this device's own state (sessions) does not.
    fn forget_the_device_state(&self) {
        let state = self.home.join(".state");
        if state.exists() {
            std::fs::remove_dir_all(&state).unwrap();
        }
    }

    /// Found the project as `a@b.c` with git config naming them, enrol
    /// them, and register an AI member so that every write needs proof.
    fn a_project_with_an_ai_member(&self) {
        self.git_config_says("a@b.c");
        let init = self.joy(&["init", "--name", "Ledger", "--acronym", "LG"]);
        assert!(init.status.success(), "{}", text(&init));
        let auth = self.joy(&["auth", "init", "--passphrase", PASSPHRASE]);
        assert!(auth.status.success(), "{}", text(&auth));
        let ai = self.joy(&[
            "project",
            "member",
            "add",
            "ai:claude@joy",
            "--capabilities",
            "implement",
            "--passphrase",
            PASSPHRASE,
        ]);
        assert!(ai.status.success(), "{}", text(&ai));
    }

    fn signed_in(&self) -> bool {
        let status = self.joy(&["auth", "status"]);
        status.status.success() && text(&status).contains("your session in this terminal")
    }

    fn item_count(&self) -> usize {
        std::fs::read_dir(self.root.join(".joy").join("items"))
            .map(|d| d.count())
            .unwrap_or(0)
    }
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// The guard group: `joy add` in a project with an AI member. Signed in,
/// nothing is asked; signed out with `--passphrase`, the item is created
/// and the session is made; signed out without, the refusal names both
/// ways and nothing is created.
#[test]
fn a_guarded_write_takes_the_passphrase_and_makes_the_session() {
    let machine = Machine::new();
    machine.a_project_with_an_ai_member();

    // (1) The session `auth init` made: nothing is asked.
    assert!(machine.signed_in());
    let add = machine.joy(&["add", "task", "With a session"]);
    assert!(add.status.success(), "{}", text(&add));
    assert!(!text(&add).contains("Authenticated as"), "{}", text(&add));

    // (4) Signed out, no passphrase, no terminal: the one refusal.
    machine.forget_the_device_state();
    let refused = machine.joy(&["add", "task", "Without anything"]);
    assert!(!refused.status.success(), "{}", text(&refused));
    assert!(
        text(&refused).contains("run `joy auth`") && text(&refused).contains("--passphrase"),
        "the refusal names both ways: {}",
        text(&refused)
    );
    assert_eq!(machine.item_count(), 1, "nothing was created");
    assert!(!machine.signed_in());

    // (2) Signed out, the passphrase on the call: the item is created,
    // and the session `joy auth` would make is made.
    let with_passphrase = machine.joy(&[
        "add",
        "task",
        "With the passphrase",
        "--passphrase",
        PASSPHRASE,
    ]);
    assert!(
        with_passphrase.status.success(),
        "{}",
        text(&with_passphrase)
    );
    assert!(
        text(&with_passphrase).contains("Authenticated as a@b.c. Session active (24h)."),
        "{}",
        text(&with_passphrase)
    );
    assert_eq!(machine.item_count(), 2);
    assert!(machine.signed_in(), "the passphrase made the session");
    let again = machine.joy(&["add", "task", "And again"]);
    assert!(again.status.success(), "{}", text(&again));
    assert!(
        !text(&again).contains("Authenticated as"),
        "nothing asked the second time"
    );

    // A wrong passphrase creates nothing and signs nobody in.
    machine.forget_the_device_state();
    let wrong = machine.joy(&["add", "task", "Wrong", "--passphrase", "not it at all"]);
    assert!(!wrong.status.success(), "{}", text(&wrong));
    assert_eq!(machine.item_count(), 3);
    assert!(!machine.signed_in());
}

/// `--user` with the passphrase proves that one call and remembers
/// nothing: the write is theirs, and nobody is signed in afterwards.
#[test]
fn user_with_the_passphrase_proves_one_call_and_makes_no_session() {
    let machine = Machine::new();
    machine.a_project_with_an_ai_member();
    machine.forget_the_device_state();

    let once = machine.joy(&[
        "add",
        "--user",
        "a@b.c",
        "--passphrase",
        PASSPHRASE,
        "task",
        "Once",
    ]);
    assert!(once.status.success(), "{}", text(&once));
    assert!(!text(&once).contains("Session active"), "{}", text(&once));
    assert_eq!(machine.item_count(), 1);
    assert!(!machine.signed_in());

    // Without the passphrase the name alone is no proof here.
    let unproven = machine.joy(&["add", "--user", "a@b.c", "task", "Unproven"]);
    assert!(!unproven.status.success(), "{}", text(&unproven));
    assert!(
        text(&unproven).contains("run `joy auth`"),
        "{}",
        text(&unproven)
    );
    assert_eq!(machine.item_count(), 1);
}

/// The crypt group: `joy crypt add` with the passphrase makes the
/// session, and `joy ls` reads the encrypted item back with nothing
/// asked, non-interactively, because the session carries the seed.
#[test]
fn the_crypt_commands_go_through_the_same_gate() {
    let machine = Machine::new();
    machine.git_config_says("a@b.c");
    let init = machine.joy(&["init", "--name", "Ledger", "--acronym", "LG"]);
    assert!(init.status.success(), "{}", text(&init));
    let auth = machine.joy(&["auth", "init", "--passphrase", PASSPHRASE]);
    assert!(auth.status.success(), "{}", text(&auth));
    let add = machine.joy(&["add", "task", "Secret thing"]);
    assert!(add.status.success(), "{}", text(&add));
    machine.forget_the_device_state();

    // Signed out, no passphrase, no terminal: the refusal.
    let refused = machine.joy(&["crypt", "add", "LG-0001"]);
    assert!(!refused.status.success(), "{}", text(&refused));
    assert!(
        text(&refused).contains("run `joy auth`"),
        "{}",
        text(&refused)
    );

    // With the passphrase: encrypted, and signed in.
    let encrypted = machine.joy(&["crypt", "add", "LG-0001", "--passphrase", PASSPHRASE]);
    assert!(encrypted.status.success(), "{}", text(&encrypted));
    assert!(machine.signed_in(), "the passphrase made the session");

    // The session carries the seed: reading asks nothing.
    let ls = machine.joy(&["ls"]);
    assert!(ls.status.success(), "{}", text(&ls));
    assert!(
        text(&ls).contains("Secret thing"),
        "the item is decrypted: {}",
        text(&ls)
    );
    let show = machine.joy(&["show", "LG-0001"]);
    assert!(show.status.success(), "{}", text(&show));
    assert!(text(&show).contains("Secret thing"), "{}", text(&show));
}

/// The member group: `joy project member add` with the passphrase makes
/// the session too, and the next manage action asks nothing.
#[test]
fn the_member_commands_go_through_the_same_gate() {
    let machine = Machine::new();
    machine.git_config_says("a@b.c");
    let init = machine.joy(&["init", "--name", "Ledger", "--acronym", "LG"]);
    assert!(init.status.success(), "{}", text(&init));
    let auth = machine.joy(&["auth", "init", "--passphrase", PASSPHRASE]);
    assert!(auth.status.success(), "{}", text(&auth));
    machine.forget_the_device_state();

    let refused = machine.joy(&["project", "member", "add", "b@c.d"]);
    assert!(!refused.status.success(), "{}", text(&refused));
    assert!(
        text(&refused).contains("run `joy auth`"),
        "{}",
        text(&refused)
    );

    let added = machine.joy(&[
        "project",
        "member",
        "add",
        "b@c.d",
        "--passphrase",
        PASSPHRASE,
    ]);
    assert!(added.status.success(), "{}", text(&added));
    assert!(machine.signed_in());
    let edited = machine.joy(&[
        "project",
        "member",
        "edit",
        "b@c.d",
        "--capabilities",
        "test",
    ]);
    assert!(
        edited.status.success(),
        "nothing asked with the session: {}",
        text(&edited)
    );
}

/// `--passphrase-stdin` is the same door for the pipe.
#[test]
fn the_passphrase_can_come_from_stdin() {
    use std::io::Write;
    let machine = Machine::new();
    machine.a_project_with_an_ai_member();
    machine.forget_the_device_state();

    let mut child = machine
        .command(&["add", "task", "Piped", "--passphrase-stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("joy runs");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{PASSPHRASE}\n").as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", text(&output));
    assert!(machine.signed_in());
}
