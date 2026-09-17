// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The identity call sites of the CLI (D3.9 and package J11 of the forge
//! connection NG design, JOY-02A0-6E).
//!
//! Every command that needs to know who is acting asks
//! `joy_core::identity::resolve_identity` through
//! `identity::acting_member_key`: the delegation session first, then the
//! member this device pinned, then git config, and git config only as
//! the prefill a person is offered. These tests drive the real binary,
//! because the question is what a command does on a machine, and the
//! machine is what they take away.

use std::path::PathBuf;
use std::process::Output;

/// A machine: a checkout, a home with its own state and config
/// directories, and no git identity from anywhere else.
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

    /// Run `joy` on this machine, with an environment that has no git
    /// identity in it: git's own system config is switched off, HOME and
    /// the XDG directories are the machine's own, and the addresses git
    /// falls back to are removed.
    fn joy(&self, args: &[&str]) -> Output {
        self.joy_with_session(args, None)
    }

    fn joy_with_session(&self, args: &[&str], session: Option<&str>) -> Output {
        let mut command = joy_process::command(env!("CARGO_BIN_EXE_joy"));
        command
            .args(args)
            .current_dir(&self.root)
            .env("HOME", &self.home)
            .env("XDG_STATE_HOME", self.home.join(".state"))
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("JOY_PASSPHRASE")
            .env_remove("GIT_AUTHOR_EMAIL")
            .env_remove("GIT_COMMITTER_EMAIL")
            .env_remove("EMAIL");
        match session {
            Some(value) => command.env("JOY_SESSION", value),
            None => command.env_remove("JOY_SESSION"),
        };
        command.output().expect("joy runs")
    }

    /// Write a global `user.email`, the way `git config --global` would.
    fn git_config_says(&self, email: &str) {
        std::fs::write(
            self.home.join(".gitconfig"),
            format!("[user]\n\temail = {email}\n\tname = Somebody\n"),
        )
        .unwrap();
    }

    fn forget_the_git_config(&self) {
        std::fs::remove_file(self.home.join(".gitconfig")).unwrap();
    }
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

const PASSPHRASE: &str = "correct horse battery staple";
const SECOND_PASSPHRASE: &str = "second pass phrase entirely";

/// Found the project and enrol the founder, so the member is known.
fn found_and_enrol(machine: &Machine) {
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
    let auth = machine.joy(&["auth", "init", "--passphrase", PASSPHRASE]);
    assert!(auth.status.success(), "{}", text(&auth));
}

/// One command of the script, with what it printed. `label` names the
/// command so a failed comparison says which one disagreed.
struct Step {
    label: &'static str,
    ok: bool,
    text: String,
}

/// Every group of identity call sites D3.9 lists, in one run: auth,
/// crypt, board (the item write), project and the event log behind them.
/// The passphrase commands come last because they end the session.
fn identity_script(machine: &Machine) -> Vec<Step> {
    let commands: Vec<(&'static str, Vec<&str>)> = vec![
        ("auth status", vec!["auth", "status"]),
        ("add an item", vec!["add", "task", "First thing"]),
        (
            "crypt add",
            vec!["crypt", "add", "LG-0001", "--passphrase", PASSPHRASE],
        ),
        ("crypt status", vec!["crypt", "status"]),
        (
            "member add",
            vec![
                "project",
                "member",
                "add",
                "b@c.d",
                "--passphrase",
                PASSPHRASE,
            ],
        ),
        (
            "member edit",
            vec![
                "project",
                "member",
                "edit",
                "b@c.d",
                "--capabilities",
                "test",
                "--passphrase",
                PASSPHRASE,
            ],
        ),
        (
            "member rm",
            vec![
                "project",
                "member",
                "rm",
                "b@c.d",
                "--passphrase",
                PASSPHRASE,
            ],
        ),
        (
            "auth passphrase",
            vec![
                "auth",
                "passphrase",
                "--passphrase",
                PASSPHRASE,
                "--new-passphrase",
                SECOND_PASSPHRASE,
            ],
        ),
        ("deauth", vec!["deauth"]),
        (
            "auth again",
            vec!["auth", "--passphrase", SECOND_PASSPHRASE],
        ),
        (
            "auth recover",
            vec![
                "auth",
                "recover",
                "--regenerate-key",
                "--passphrase",
                SECOND_PASSPHRASE,
            ],
        ),
    ];
    commands
        .into_iter()
        .map(|(label, args)| {
            let output = machine.joy(&args);
            Step {
                label,
                ok: output.status.success(),
                text: redact(&text(&output)),
            }
        })
        .collect()
}

/// Drop the lines that differ between two equal runs by nature: the
/// countdown of a session, the one-time password of a new member and the
/// recovery key. Everything else must match line for line.
fn redact(output: &str) -> String {
    output
        .lines()
        .filter(|line| {
            !line.contains("Expires:")
                && !line.contains("One-time password")
                && !line.contains("--otp")
                && !line.contains("joy_r_")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The acceptance of J11, first half: every joy command that needs an
/// identity works in a repository with no git config once the member is
/// known.
#[test]
fn every_identity_command_works_without_a_git_config() {
    let machine = Machine::new();
    found_and_enrol(&machine);

    for step in identity_script(&machine) {
        assert!(step.ok, "`joy {}` failed: {}", step.label, step.text);
    }

    // The commands named the founder, not an empty string and not an
    // opaque id in an open mode project.
    let status = machine.joy(&["auth", "status"]);
    assert!(
        text(&status).contains("a@b.c"),
        "the session names the founder: {}",
        text(&status)
    );
}

/// The acceptance of J11, second half: removing `user.email` from git
/// config in a joy project changes no joy command's behaviour.
///
/// Two identical machines run the identical script. One keeps the git
/// config it was founded with; the other loses it the moment the member
/// is known. Line for line, the two runs say the same thing.
#[test]
fn removing_user_email_changes_no_command() {
    let with_config = Machine::new();
    with_config.git_config_says("a@b.c");
    found_and_enrol(&with_config);

    let without_config = Machine::new();
    without_config.git_config_says("a@b.c");
    found_and_enrol(&without_config);
    without_config.forget_the_git_config();

    let kept = identity_script(&with_config);
    let removed = identity_script(&without_config);

    assert_eq!(kept.len(), removed.len());
    for (kept, removed) in kept.iter().zip(removed.iter()) {
        assert_eq!(kept.label, removed.label);
        assert_eq!(
            (kept.ok, kept.text.as_str()),
            (removed.ok, removed.text.as_str()),
            "`joy {}` behaved differently without a git config",
            kept.label
        );
    }
    // And the script really did something: every step of it succeeded.
    for step in &removed {
        assert!(step.ok, "`joy {}` failed: {}", step.label, step.text);
    }
}

/// The order of D3.9, at its top: a delegation session outranks git
/// config. `joy deauth` under an AI session ends THAT session, and the
/// operator whose address the git config carries keeps theirs.
#[test]
fn a_delegation_session_outranks_the_git_config() {
    let machine = Machine::new();
    machine.git_config_says("a@b.c");
    found_and_enrol(&machine);

    let add = machine.joy(&[
        "project",
        "member",
        "add",
        "ai:claude@joy",
        "--passphrase",
        PASSPHRASE,
    ]);
    assert!(add.status.success(), "{}", text(&add));

    let issued = machine.joy(&[
        "auth",
        "token",
        "add",
        "ai:claude@joy",
        "--passphrase",
        PASSPHRASE,
        "--json",
    ]);
    assert!(issued.status.success(), "{}", text(&issued));
    let token = json_string(&text(&issued), "token");

    let redeemed = machine.joy(&["auth", "--token", &token, "--json"]);
    assert!(redeemed.status.success(), "{}", text(&redeemed));
    let session_env = json_string(&text(&redeemed), "session_env");

    let deauth = machine.joy_with_session(&["deauth"], Some(&session_env));
    assert!(deauth.status.success(), "{}", text(&deauth));
    assert!(
        text(&deauth).contains("ai:claude@joy"),
        "the AI's own session is the one that ends: {}",
        text(&deauth)
    );

    // The operator's session survived it, so the AI did not act as the
    // person the git config names.
    let status = machine.joy(&["auth", "status"]);
    assert!(
        text(&status).contains("a@b.c") && !text(&status).contains("No active session"),
        "{}",
        text(&status)
    );
}

/// The one boundary of the criterion above, named rather than left to be
/// discovered: with no session left, no pin and no git config, joy says
/// it does not know who is acting and names the remedy. `--user` pins the
/// member again, and nothing needs git config afterwards.
#[test]
fn without_a_session_a_pin_or_a_config_joy_names_the_remedy() {
    let machine = Machine::new();
    machine.git_config_says("a@b.c");
    found_and_enrol(&machine);
    // Enrolment happened while git config named the founder, so this
    // device keeps no pin (that is the rule of `pin_acting_member`).
    machine.forget_the_git_config();

    let deauth = machine.joy(&["deauth"]);
    assert!(deauth.status.success(), "{}", text(&deauth));

    let refused = machine.joy(&["auth", "--passphrase", PASSPHRASE]);
    assert!(!refused.status.success(), "{}", text(&refused));
    assert!(
        text(&refused).contains("this project does not know who you are"),
        "{}",
        text(&refused)
    );
    assert!(
        text(&refused).contains("--user <address>"),
        "the refusal names the remedy: {}",
        text(&refused)
    );

    let named = machine.joy(&["auth", "--user", "a@b.c", "--passphrase", PASSPHRASE]);
    assert!(named.status.success(), "{}", text(&named));

    // Authenticating pinned the member, so the next command needs
    // neither a name nor a git config.
    let status = machine.joy(&["auth", "status"]);
    assert!(text(&status).contains("a@b.c"), "{}", text(&status));
}

/// Pull one string field out of a `--json` payload without a JSON parser
/// in the dev dependencies. The payloads here are machine written and
/// carry no escapes in these fields.
fn json_string(payload: &str, field: &str) -> String {
    let needle = format!("\"{field}\":\"");
    let start = payload
        .find(&needle)
        .unwrap_or_else(|| panic!("{field} in {payload}"))
        + needle.len();
    let rest = &payload[start..];
    let end = rest.find('"').expect("the field ends");
    rest[..end].to_string()
}
