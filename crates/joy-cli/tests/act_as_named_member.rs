// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Working without a git identity, and acting as somebody other than
//! the one git config names (operator decision 2026-09-27).
//!
//! Three things had to become true. `--user <address>` is a global
//! flag: on any command it names the member this one call acts as, and
//! nothing is remembered; `joy ai init --user` therefore exists, which
//! the refusal used to suggest while the flag did not. `joy auth`
//! (with or without `--user`) makes THE session of this project on this
//! device, replacing whoever was signed in before, and that session is
//! read before git config, so every later command in the same terminal
//! acts as that member. And the refusal is one line naming exactly
//! those two ways.
//!
//! These drive the real binary, the way `identity_call_sites.rs` does,
//! because the question is what a person at a terminal gets.

// Nothing of the developer's shell and session reaches this binary
// (JOY-02BB-C7).
joy_test_env::isolate!();

use std::path::PathBuf;
use std::process::Output;

const PASSPHRASE: &str = "correct horse battery staple";
const SECOND_PASSPHRASE: &str = "second pass phrase entirely";

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

    fn joy(&self, args: &[&str]) -> Output {
        joy_test_env::command(env!("CARGO_BIN_EXE_joy"))
            .args(args)
            .current_dir(&self.root)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("HOMEDRIVE", "")
            .env("HOMEPATH", "")
            .env("XDG_STATE_HOME", self.home.join(".state"))
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("joy runs")
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

    /// Found the project as `a@b.c`, naming the founder at both steps,
    /// which is what a founder with no git config has to do.
    fn found_and_enrol(&self) {
        let init = self.joy(&[
            "init",
            "--name",
            "Ledger",
            "--acronym",
            "LG",
            "--user",
            "a@b.c",
        ]);
        assert!(init.status.success(), "{}", text(&init));
        let auth = self.joy(&[
            "auth",
            "init",
            "--user",
            "a@b.c",
            "--passphrase",
            PASSPHRASE,
        ]);
        assert!(auth.status.success(), "{}", text(&auth));
    }

    /// Invite `email` and redeem the invitation as them, the real
    /// two-sided flow, ending with `email` enrolled, holding
    /// [`SECOND_PASSPHRASE`] and signed in at this terminal. The git
    /// config is left exactly as it was: that is the point of the cases
    /// that call this.
    fn a_second_enrolled_member(&self, email: &str) {
        let invited = self.joy(&[
            "project",
            "member",
            "add",
            email,
            "--capabilities",
            "all",
            "--passphrase",
            PASSPHRASE,
        ]);
        assert!(invited.status.success(), "{}", text(&invited));
        let otp = text(&invited)
            .split_whitespace()
            .find(|word| is_a_one_time_password(word))
            .expect("the invitation prints a one-time password")
            .to_string();
        let redeemed = self.joy(&[
            "auth",
            "--otp",
            &otp,
            "--user",
            email,
            "--passphrase",
            SECOND_PASSPHRASE,
        ]);
        assert!(redeemed.status.success(), "{}", text(&redeemed));
    }

    /// The `created_by` of the one item in the project, raw as stored.
    fn the_actor_of_the_only_item(&self) -> String {
        let items = self.root.join(".joy").join("items");
        let mut files: Vec<_> = std::fs::read_dir(&items)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(files.len(), 1, "one item: {files:?}");
        let text = std::fs::read_to_string(files.pop().unwrap()).unwrap();
        text.lines()
            .find_map(|line| line.strip_prefix("created_by: "))
            .expect("the item says who created it")
            .to_string()
    }
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// A one-time password as `joy project member add` prints it: three
/// groups of four upper-case letters and digits, joined by dashes.
fn is_a_one_time_password(word: &str) -> bool {
    let groups: Vec<&str> = word.split('-').collect();
    groups.len() == 3
        && groups.iter().all(|group| {
            group.len() == 4
                && group
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        })
}

/// The defect as reported: `joy ai init` on a machine with no git
/// identity refused with a sentence naming `--user`, and `joy ai init
/// --user` was not a command. Now it is, and it does the whole job on
/// such a machine: it enrols the founder who has no passphrase yet and
/// registers the tool member under their attestation. It leaves NO
/// session behind: only `joy auth` does that.
#[test]
fn ai_init_user_sets_up_the_tools_with_no_git_identity() {
    let machine = Machine::new();
    let init = machine.joy(&[
        "init",
        "--name",
        "Ledger",
        "--acronym",
        "LG",
        "--user",
        "a@b.c",
    ]);
    assert!(init.status.success(), "{}", text(&init));

    // Without a name, and with nobody signed in, the refusal is one line
    // naming the two remedies, both of which exist.
    let refused = machine.joy(&["ai", "init", "--tool", "claude", "--passphrase", PASSPHRASE]);
    assert!(!refused.status.success(), "{}", text(&refused));
    assert!(
        text(&refused).contains("not signed in: run `joy auth`, or pass `--user <address>`"),
        "{}",
        text(&refused)
    );

    let set_up = machine.joy(&[
        "ai",
        "init",
        "--tool",
        "claude",
        "--user",
        "a@b.c",
        "--passphrase",
        PASSPHRASE,
    ]);
    assert!(set_up.status.success(), "{}", text(&set_up));
    assert!(
        text(&set_up).contains("Authentication initialized for a@b.c"),
        "the founder is enrolled on the way: {}",
        text(&set_up)
    );
    assert!(
        !text(&set_up).contains("Session active"),
        "no session is made: {}",
        text(&set_up)
    );
    assert!(
        text(&set_up).contains("ai:claude@joy"),
        "the tool member is registered: {}",
        text(&set_up)
    );
    let project = std::fs::read_to_string(machine.root.join(".joy/project.yaml")).unwrap();
    assert!(project.contains("ai:claude@joy"), "{project}");

    // Nothing was remembered: the next command needs the name again.
    // A write the guard protects (the project has an AI member now)
    // still wants a session or the passphrase at the guard, which is
    // the follow-up's job; a command that unlocks with the passphrase
    // itself, like a second `ai init`, acts as the founder for that one
    // call. Still no session afterwards.
    let status = machine.joy(&["auth", "status"]);
    assert!(!status.status.success(), "{}", text(&status));
    assert!(text(&status).contains("not signed in"), "{}", text(&status));
    let unnamed = machine.joy(&["add", "task", "First thing"]);
    assert!(!unnamed.status.success(), "{}", text(&unnamed));
    assert!(
        text(&unnamed).contains("not signed in"),
        "{}",
        text(&unnamed)
    );
    let again = machine.joy(&[
        "ai",
        "init",
        "--tool",
        "qwen",
        "--user",
        "a@b.c",
        "--passphrase",
        PASSPHRASE,
    ]);
    assert!(again.status.success(), "{}", text(&again));
    let project = std::fs::read_to_string(machine.root.join(".joy/project.yaml")).unwrap();
    assert!(project.contains("ai:qwen@joy"), "{project}");
    let status = machine.joy(&["auth", "status"]);
    assert!(
        !status.status.success(),
        "still nobody signed in: {}",
        text(&status)
    );
    assert!(!machine.home.join(".gitconfig").exists());
}

/// The second machine of a member who is already enrolled: no session
/// here, no git config, and `joy ai init --user` does the setup with the
/// passphrase it needs for the attestation anyway, still without a
/// session.
#[test]
fn ai_init_user_on_a_new_machine_leaves_no_session_either() {
    let machine = Machine::new();
    machine.found_and_enrol();
    machine.forget_the_device_state();

    let set_up = machine.joy(&[
        "ai",
        "init",
        "--tool",
        "claude",
        "--user",
        "a@b.c",
        "--passphrase",
        PASSPHRASE,
    ]);
    assert!(set_up.status.success(), "{}", text(&set_up));
    assert!(text(&set_up).contains("ai:claude@joy"), "{}", text(&set_up));
    assert!(
        !text(&set_up).contains("Authenticated as"),
        "no sign-in on the way: {}",
        text(&set_up)
    );
    let status = machine.joy(&["auth", "status"]);
    assert!(!status.status.success(), "{}", text(&status));
}

/// A wrong passphrase on that path is a refusal, not a half-done setup:
/// nothing is registered and nobody is signed in.
#[test]
fn ai_init_user_with_the_wrong_passphrase_registers_nothing() {
    let machine = Machine::new();
    machine.found_and_enrol();
    machine.forget_the_device_state();

    let refused = machine.joy(&[
        "ai",
        "init",
        "--tool",
        "claude",
        "--user",
        "a@b.c",
        "--passphrase",
        "not the passphrase at all",
    ]);
    assert!(!refused.status.success(), "{}", text(&refused));
    let project = std::fs::read_to_string(machine.root.join(".joy/project.yaml")).unwrap();
    assert!(!project.contains("ai:claude@joy"), "{project}");
    let status = machine.joy(&["auth", "status"]);
    assert!(!status.status.success(), "{}", text(&status));
}

/// `joy auth` makes THE session: a second person signing in at a
/// checkout whose git config names the founder replaces the founder's
/// session and acts as themselves from then on, until the founder signs
/// in again, which replaces theirs in turn. The git config is never
/// touched, and a bare `joy auth` reads it, not the session.
#[test]
fn joy_auth_makes_the_one_session_whatever_the_git_config_says() {
    let machine = Machine::new();
    machine.git_config_says("a@b.c");
    machine.found_and_enrol();
    machine.a_second_enrolled_member("b@c.d");

    // b redeemed last, so b is signed in; the config still says a@b.c,
    // and the write is b's.
    let add = machine.joy(&["add", "task", "First thing"]);
    assert!(add.status.success(), "{}", text(&add));
    assert_eq!(machine.the_actor_of_the_only_item(), "b@c.d");
    let status = machine.joy(&["auth", "status"]);
    assert!(status.status.success(), "{}", text(&status));
    assert!(
        text(&status).contains("b@c.d")
            && text(&status).contains("Source:     your session in this terminal"),
        "{}",
        text(&status)
    );

    // A bare `joy auth` signs in the person git config names, and b's
    // session is gone with it.
    let founder = machine.joy(&["auth", "--passphrase", PASSPHRASE]);
    assert!(founder.status.success(), "{}", text(&founder));
    assert!(
        text(&founder).contains("Authenticated as a@b.c"),
        "{}",
        text(&founder)
    );
    let status = machine.joy(&["auth", "status"]);
    assert!(status.status.success(), "{}", text(&status));
    assert!(text(&status).contains("a@b.c"), "{}", text(&status));

    // Signing out leaves nobody signed in: the config names the founder,
    // unauthenticated, and b's earlier session did not come back.
    let deauth = machine.joy(&["deauth"]);
    assert!(deauth.status.success(), "{}", text(&deauth));
    let status = machine.joy(&["auth", "status"]);
    assert!(!status.status.success(), "{}", text(&status));
    assert!(
        text(&status).contains("a@b.c")
            && text(&status).contains("Source:     git config user.email"),
        "{}",
        text(&status)
    );

    let config = std::fs::read_to_string(machine.home.join(".gitconfig")).unwrap();
    assert!(config.contains("a@b.c"), "the git config was never touched");
}

/// `--user` on any other command is a name for that one call: the
/// command acts as that member, unproven like git config, and the
/// session of whoever is signed in stands untouched afterwards.
#[test]
fn user_on_any_other_command_acts_once_and_remembers_nothing() {
    let machine = Machine::new();
    machine.git_config_says("a@b.c");
    machine.found_and_enrol();
    machine.a_second_enrolled_member("b@c.d");
    let founder = machine.joy(&["auth", "--user", "a@b.c", "--passphrase", PASSPHRASE]);
    assert!(founder.status.success(), "{}", text(&founder));

    let theirs = machine.joy(&["add", "--user", "b@c.d", "task", "Theirs"]);
    assert!(theirs.status.success(), "{}", text(&theirs));
    assert_eq!(machine.the_actor_of_the_only_item(), "b@c.d");

    // The founder is still the one signed in.
    let status = machine.joy(&["auth", "status"]);
    assert!(status.status.success(), "{}", text(&status));
    assert!(
        text(&status).contains("a@b.c")
            && text(&status).contains("Source:     your session in this terminal"),
        "{}",
        text(&status)
    );
    // And `joy auth status --user` says whom that call would act as.
    let named = machine.joy(&["auth", "status", "--user", "b@c.d"]);
    assert!(
        text(&named).contains("b@c.d") && text(&named).contains("Source:     --user on this call"),
        "{}",
        text(&named)
    );
}
