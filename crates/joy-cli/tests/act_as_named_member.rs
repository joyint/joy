// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Working without a git identity, and acting as somebody other than
//! the one git config names (operator decision 2026-09-27).
//!
//! Two things had to become true. `joy ai init` takes `--user <address>`
//! like `joy init` and `joy auth` do, so the person who sets the AI
//! tools up can say who they are on a machine with no git config, or
//! set them up as a member other than the one the config names. And a
//! sign-in by name sticks: the session `joy auth --user` leaves behind
//! is read before git config, so every later command in the same
//! terminal acts as that member. Before this, the refusal named `--user`
//! as the remedy while `joy ai init` had no such flag, and a sign-in by
//! name changed nothing for the next command.
//!
//! These drive the real binary, the way `identity_call_sites.rs` does,
//! because the question is what a person at a terminal gets.

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
        joy_process::command(env!("CARGO_BIN_EXE_joy"))
            .args(args)
            .current_dir(&self.root)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("HOMEDRIVE", "")
            .env("HOMEPATH", "")
            .env("XDG_STATE_HOME", self.home.join(".state"))
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("JOY_SESSION")
            .env_remove("JOY_PASSPHRASE")
            .env_remove("GIT_AUTHOR_EMAIL")
            .env_remove("GIT_COMMITTER_EMAIL")
            .env_remove("EMAIL")
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
/// such a machine: it enrols the founder who has no passphrase yet,
/// registers the tool member under their attestation, and leaves the
/// session that names them for the commands that follow.
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

    // Without a name, and with nobody signed in, the refusal names a
    // remedy that exists: a sign-in by name.
    let refused = machine.joy(&["ai", "init", "--tool", "claude", "--passphrase", PASSPHRASE]);
    assert!(!refused.status.success(), "{}", text(&refused));
    assert!(
        text(&refused).contains("this project does not know who you are"),
        "{}",
        text(&refused)
    );
    assert!(
        text(&refused).contains("joy auth --user <address>"),
        "the refusal names the remedy: {}",
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
        text(&set_up).contains("ai:claude@joy"),
        "the tool member is registered: {}",
        text(&set_up)
    );
    let project = std::fs::read_to_string(machine.root.join(".joy/project.yaml")).unwrap();
    assert!(project.contains("ai:claude@joy"), "{project}");

    // The founder is signed in now; the next command needs no name and
    // no git config, and says where its answer came from.
    let add = machine.joy(&["add", "task", "First thing"]);
    assert!(add.status.success(), "{}", text(&add));
    assert_eq!(machine.the_actor_of_the_only_item(), "a@b.c");
    let status = machine.joy(&["auth", "status"]);
    assert!(status.status.success(), "{}", text(&status));
    assert!(
        text(&status).contains("Source:     your session in this terminal"),
        "{}",
        text(&status)
    );
    assert!(!machine.home.join(".gitconfig").exists());
}

/// The second machine of a member who is already enrolled: no session
/// here yet, no git config, and `joy ai init --user` signs them in with
/// the passphrase it needs anyway for the attestation, instead of
/// telling them to do so first.
#[test]
fn ai_init_user_signs_an_enrolled_member_in_on_a_new_machine() {
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
    assert!(
        text(&set_up).contains("Authenticated as a@b.c"),
        "the named member is signed in on the way: {}",
        text(&set_up)
    );
    assert!(text(&set_up).contains("ai:claude@joy"), "{}", text(&set_up));

    let status = machine.joy(&["auth", "status"]);
    assert!(status.status.success(), "{}", text(&status));
    assert!(text(&status).contains("a@b.c"), "{}", text(&status));
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

/// Despite a git config that names the founder, a second person signs
/// in by name at the same checkout and acts as themselves: the item
/// they create is theirs, `joy auth status` names them, and the founder
/// takes over again by signing in by name in turn. The git config is
/// never touched.
#[test]
fn a_sign_in_by_name_acts_as_that_member_despite_the_git_config() {
    let machine = Machine::new();
    machine.git_config_says("a@b.c");
    machine.found_and_enrol();
    machine.a_second_enrolled_member("b@c.d");

    // The newest sign-in at this terminal is b@c.d; the config still
    // says a@b.c, and the write is b's.
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

    // The founder signs in by name and is the one acting again, their
    // older session renewed and therefore the newest.
    let founder = machine.joy(&["auth", "--user", "a@b.c", "--passphrase", PASSPHRASE]);
    assert!(founder.status.success(), "{}", text(&founder));
    let status = machine.joy(&["auth", "status"]);
    assert!(status.status.success(), "{}", text(&status));
    assert!(text(&status).contains("a@b.c"), "{}", text(&status));

    // Signing out ends the founder's session; b's still stands, so b is
    // acting. Signing out once more leaves the git config, which names
    // the founder, unauthenticated.
    let deauth = machine.joy(&["deauth"]);
    assert!(deauth.status.success(), "{}", text(&deauth));
    let status = machine.joy(&["auth", "status"]);
    assert!(status.status.success(), "{}", text(&status));
    assert!(text(&status).contains("b@c.d"), "{}", text(&status));
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
