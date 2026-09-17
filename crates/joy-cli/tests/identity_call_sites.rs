// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The identity call sites of the CLI (D3.9 and package J11 of the forge
//! connection NG design, JOY-02A0-6E).
//!
//! Every command that needs to know who is acting asks
//! `joy_core::identity::resolve_identity`, through
//! `identity::acting_member_key` where the actor is whoever acts, and
//! through `identity::acting_human_key` where a passphrase is needed and
//! the answer must be a person. Both read the delegation session first,
//! then the member this device pinned, then git config, and git config
//! only as the prefill a person is offered. These tests drive the real
//! binary, because the question is what a command does on a machine, and
//! the machine is what they take away.

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

    /// Run `joy` with `JOY_PASSPHRASE` in the environment, the way a
    /// script or the desktop's sidecar does. It is the door the member
    /// resolver uses to open an anonymous project's member map without a
    /// session, and it asks who acts here to know whose seed to derive.
    fn joy_with_a_passphrase_in_the_environment(&self, args: &[&str]) -> Output {
        let mut command = self.base_command(args);
        command.env_remove("JOY_SESSION");
        command.env("JOY_PASSPHRASE", PASSPHRASE);
        command.output().expect("joy runs")
    }

    fn joy_with_session(&self, args: &[&str], session: Option<&str>) -> Output {
        let mut command = self.base_command(args);
        command.env_remove("JOY_PASSPHRASE");
        match session {
            Some(value) => command.env("JOY_SESSION", value),
            None => command.env_remove("JOY_SESSION"),
        };
        command.output().expect("joy runs")
    }

    fn base_command(&self, args: &[&str]) -> std::process::Command {
        let mut command = joy_process::command(env!("CARGO_BIN_EXE_joy"));
        command
            .args(args)
            .current_dir(&self.root)
            .env("HOME", &self.home)
            .env("XDG_STATE_HOME", self.home.join(".state"))
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_AUTHOR_EMAIL")
            .env_remove("GIT_COMMITTER_EMAIL")
            .env_remove("EMAIL");
        command
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

    /// Model another machine, or a fresh clone: the project file travels,
    /// this device's own state does not. Both the sessions and the member
    /// pin live in it.
    fn forget_the_device_state(&self) {
        let state = self.home.join(".state");
        if state.exists() {
            std::fs::remove_dir_all(&state).unwrap();
        }
    }

    /// Invite `email` and redeem the invitation as them: the real
    /// two-sided flow, ending with `email` enrolled, holding
    /// [`SECOND_PASSPHRASE`], and pinned as the member this device acts
    /// as. Returns the redemption's output.
    fn a_second_enrolled_member(&self, email: &str) -> Output {
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
        self.joy(&[
            "auth",
            "--otp",
            &otp,
            "--user",
            email,
            "--passphrase",
            SECOND_PASSPHRASE,
        ])
    }

    /// Register an AI member with full rights, issue a delegation token
    /// for it and redeem it. Returns the `JOY_SESSION` value.
    fn a_delegation_session(&self, ai: &str) -> String {
        let add = self.joy(&[
            "project",
            "member",
            "add",
            ai,
            "--capabilities",
            "all",
            "--passphrase",
            PASSPHRASE,
        ]);
        assert!(add.status.success(), "{}", text(&add));

        let issued = self.joy(&[
            "auth",
            "token",
            "add",
            ai,
            "--passphrase",
            PASSPHRASE,
            "--json",
        ]);
        assert!(issued.status.success(), "{}", text(&issued));
        let token = json_string(&text(&issued), "token");

        let redeemed = self.joy(&["auth", "--token", &token, "--json"]);
        assert!(redeemed.status.success(), "{}", text(&redeemed));
        json_string(&text(&redeemed), "session_env")
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
/// The address the founder is named by. In an anonymous project it is
/// the last place it is ever written: from `init` on, the member map
/// knows the founder by an opaque id alone.
const FOUNDER: &str = "a@b.c";

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

/// Found the project in anonymous mode (ADR-042). The founder identity is
/// established by `init` itself, because the very first committed
/// project.yaml has to be keyed by the opaque id already; there is no
/// separate `auth init` step afterwards.
fn found_anonymously(machine: &Machine) {
    let init = machine.joy(&[
        "init",
        "--name",
        "Ledger",
        "--acronym",
        "LG",
        "--user",
        FOUNDER,
        "--anonymous",
        "--passphrase",
        PASSPHRASE,
    ]);
    assert!(init.status.success(), "{}", text(&init));
}

/// The `created_by` of the one item in the project, raw as it is stored.
fn the_actor_of_the_only_item(machine: &Machine) -> String {
    let items = machine.root.join(".joy").join("items");
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

/// Whether the address appears in ANY file under `.joy/`, read as bytes
/// so the encrypted members file is searched like the rest.
fn joy_dir_mentions(machine: &Machine, needle: &str) -> bool {
    fn walk(dir: &std::path::Path, needle: &[u8], found: &mut bool) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, needle, found);
            } else if std::fs::read(&path)
                .unwrap()
                .windows(needle.len())
                .any(|w| w == needle)
            {
                *found = true;
            }
        }
    }
    let mut found = false;
    walk(&machine.root.join(".joy"), needle.as_bytes(), &mut found);
    found
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
        // The AI setup path: it attests the tool member it registers
        // with the acting human's key, and it bootstraps that human's
        // authentication when there is none.
        (
            "ai init",
            vec!["ai", "init", "--tool", "claude", "--passphrase", PASSPHRASE],
        ),
        (
            "chat send",
            vec![
                "chat",
                "send",
                "general",
                "hello there",
                "--passphrase",
                PASSPHRASE,
            ],
        ),
        (
            "chat show",
            vec!["chat", "show", "general", "--passphrase", PASSPHRASE],
        ),
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

/// The identity call sites the script above cannot host, because every
/// one of them takes something away: a zone grant, a zone, a member's
/// authentication. They run in one order that leaves each of them
/// reachable, ending with the acting member's own reset.
///
/// This is where `joy auth reset` (auth.rs), `joy crypt grant`,
/// `joy crypt revoke` and `joy crypt zone rm` (crypt.rs) and
/// `joy auth delegation rotate` (auth.rs) are driven. `joy auth reset`
/// is here for a second reason: it looked the acting member up by
/// address and then by key, which failed outright in a project whose
/// member map is not keyed by an address; the move onto
/// `acting_human_key` fixed that, and this is the case that says so.
fn taking_something_away_script(machine: &Machine) -> Vec<Step> {
    let commands: Vec<(&'static str, Vec<&str>)> = vec![
        ("add an item", vec!["add", "task", "First thing"]),
        (
            "crypt add",
            vec!["crypt", "add", "LG-0001", "--passphrase", PASSPHRASE],
        ),
        (
            "crypt grant",
            vec!["crypt", "grant", "a@b.c", "--passphrase", PASSPHRASE],
        ),
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
        // A member with no access: the early return, which still had to
        // know who was asking.
        ("crypt revoke a stranger", vec!["crypt", "revoke", "b@c.d"]),
        // Resetting somebody ELSE needs the manage capability, and the
        // acting member has to be found for it to be checked at all.
        (
            "auth reset another member",
            vec!["auth", "reset", "b@c.d", "--passphrase", PASSPHRASE],
        ),
        (
            "ai member add",
            vec![
                "project",
                "member",
                "add",
                "ai:claude@joy",
                "--capabilities",
                "all",
                "--passphrase",
                PASSPHRASE,
            ],
        ),
        (
            "token add",
            vec![
                "auth",
                "token",
                "add",
                "ai:claude@joy",
                "--passphrase",
                PASSPHRASE,
            ],
        ),
        (
            "delegation rotate",
            vec![
                "auth",
                "delegation",
                "rotate",
                "ai:claude@joy",
                "--passphrase",
                PASSPHRASE,
            ],
        ),
        (
            "crypt rm",
            vec!["crypt", "rm", "LG-0001", "--passphrase", PASSPHRASE],
        ),
        ("crypt revoke myself", vec!["crypt", "revoke", "a@b.c"]),
        ("crypt zone rm", vec!["crypt", "zone", "rm", "default"]),
        (
            "auth reset myself",
            vec!["auth", "reset", "--passphrase", PASSPHRASE],
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

/// The privacy migration and the erasure behind it (`joy project set
/// privacy`, `joy project member erase`), which no other script can
/// host: the migration rekeys every human member, so the address a
/// machine was founded with stops being a member key halfway through.
///
/// This is the one command that used to be rescued by git config on a
/// machine that had one: the pin still named the address, the address
/// was no longer a key, and `member_key_for_email` found the new id from
/// the config. With git config out of the identity (J11) the migration
/// has to re-pin the acting member itself, and this script is where that
/// is checked, on a machine that has no config to fall back on.
fn privacy_migration_script(machine: &Machine) -> Vec<Step> {
    // The third field says whether this step carries `JOY_PASSPHRASE`:
    // the member resolver's own door into an anonymous member map, which
    // also has to know who acts here.
    let commands: Vec<(&'static str, Vec<&str>, bool)> = vec![
        (
            "set privacy anonymous",
            vec![
                "project",
                "set",
                "privacy",
                "anonymous",
                "--passphrase",
                PASSPHRASE,
            ],
            false,
        ),
        // The rekey invalidated the session, because it was bound to the
        // old member key. Authenticating again needs the pin the
        // migration rewrote, and no `--user`.
        (
            "auth again",
            vec!["auth", "--passphrase", PASSPHRASE],
            false,
        ),
        ("auth status", vec!["auth", "status"], false),
        // The member resolver, through its passphrase door: an opaque id
        // is shown as the person's address again, and the seed it
        // derives to open the map is the acting member's.
        (
            "member show",
            vec!["project", "member", "show", "a@b.c"],
            true,
        ),
        (
            "member erase",
            vec![
                "project",
                "member",
                "erase",
                "a@b.c",
                "--passphrase",
                PASSPHRASE,
            ],
            false,
        ),
        (
            "set privacy open",
            vec![
                "project",
                "set",
                "privacy",
                "open",
                "--passphrase",
                PASSPHRASE,
            ],
            false,
        ),
    ];
    commands
        .into_iter()
        .map(|(label, args, with_passphrase_env)| {
            let output = match with_passphrase_env {
                true => machine.joy_with_a_passphrase_in_the_environment(&args),
                false => machine.joy(&args),
            };
            Step {
                label,
                ok: output.status.success(),
                text: redact(&text(&output)),
            }
        })
        .collect()
}

/// Take out what differs between two equal runs by nature: the countdown
/// of a session, the one-time password of a new member, the recovery key,
/// a delegation token, the clock in front of a chat line and the
/// milliseconds a chat write took.
///
/// Every one of them is replaced BY VALUE, never by dropping the line it
/// sits on. Dropping lines is what hides a difference: an identity that
/// changed on a line which happens to carry the word "Expires:", or
/// inside the first sixteen characters of a chat line, would pass a
/// comparison that threw those lines away. Here every other word on such
/// a line is still compared, and the shape of what is removed is checked
/// character by character.
fn redact(output: &str) -> String {
    output
        .lines()
        .map(redact_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn redact_line(line: &str) -> String {
    let line = without_a_timestamp(line);
    // `Expires:   23h 59m`: the label stays, the countdown goes.
    if let Some((head, _)) = line.split_once("Expires:") {
        return format!("{head}Expires: <countdown>");
    }
    // `message sent (10 ms)`: how long a chat write took.
    if let Some((head, _)) = line.split_once(" (") {
        if head.trim_start().starts_with("message sent") {
            return format!("{head} (<duration>)");
        }
    }
    // Word by word for the rest, so the spacing and every other word of
    // the line are preserved exactly.
    line.split(' ')
        .map(redact_word)
        .collect::<Vec<_>>()
        .join(" ")
}

/// One word, with whatever punctuation joy printed around it kept in
/// place: a recovery key, a delegation token and a one-time password are
/// fresh on every run and nothing else about them is asserted here.
fn redact_word(word: &str) -> String {
    let bare = word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-');
    if bare.is_empty() {
        return word.to_string();
    }
    for (prefix, token) in [("joy_r_", "<recovery-key>"), ("joy_t_", "<token>")] {
        if bare.starts_with(prefix) {
            // The whole word, quotes and base64 padding included: the
            // padding is part of the value and its length is not
            // something this comparison has an opinion about.
            return token.to_string();
        }
    }
    if is_a_one_time_password(bare) {
        return word.replace(bare, "<otp>");
    }
    // An opaque member id (ADR-042) is derived from the member's
    // verify_key, so two machines that enrolled the same person hold
    // different ones. The id is replaced; an address printed beside it is
    // not, which is the whole point of comparing these runs.
    match joy_core::member_id::is_opaque_member_id(bare) {
        true => word.replace(bare, "<member-id>"),
        false => word.to_string(),
    }
}

/// A one-time password as `joy project member add` prints it: three
/// groups of four upper-case letters and digits, joined by dashes. No
/// member key has that shape: an address carries an `@` and an opaque id
/// is `m-` and one group of hex.
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

/// A chat message is printed as `<date> <time>  <member>  <text>`.
/// Replace the clock in front of it and keep the rest.
///
/// The shape is checked at every one of those sixteen characters, so a
/// line that merely begins with "20" keeps all of its text, the name in
/// it included.
fn without_a_timestamp(line: &str) -> String {
    let bytes = line.as_bytes();
    if bytes.len() < 16 {
        return line.to_string();
    }
    let digits = [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15];
    let punctuation = [(4, b'-'), (7, b'-'), (10, b' '), (13, b':')];
    let stamped = digits.iter().all(|&at| bytes[at].is_ascii_digit())
        && punctuation.iter().all(|&(at, c)| bytes[at] == c);
    match stamped {
        true => format!("<time>{}", &line[16..]),
        false => line.to_string(),
    }
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

/// Both halves of the acceptance, for the verbs that take something
/// away: they work on a machine with no git config, and removing
/// `user.email` changes nothing they print.
///
/// One comparison for both, because the script can only be run once per
/// machine: each of these commands removes the thing the next one would
/// have needed.
#[test]
fn the_verbs_that_take_something_away_ignore_the_git_config_too() {
    let with_config = Machine::new();
    with_config.git_config_says("a@b.c");
    found_and_enrol(&with_config);

    let without_config = Machine::new();
    without_config.git_config_says("a@b.c");
    found_and_enrol(&without_config);
    without_config.forget_the_git_config();

    let kept = taking_something_away_script(&with_config);
    let removed = taking_something_away_script(&without_config);

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
    for step in &removed {
        assert!(step.ok, "`joy {}` failed: {}", step.label, step.text);
    }
}

/// Both halves of the acceptance for the privacy migration: it works on
/// a machine with no git config, and removing `user.email` changes
/// nothing it prints. The opaque ids in the output are redacted by
/// value, because they are derived from each machine's own key.
#[test]
fn the_privacy_migration_keeps_knowing_who_acts() {
    let with_config = Machine::new();
    with_config.git_config_says("a@b.c");
    found_and_enrol(&with_config);

    let without_config = Machine::new();
    without_config.git_config_says("a@b.c");
    found_and_enrol(&without_config);
    without_config.forget_the_git_config();

    let kept = privacy_migration_script(&with_config);
    let removed = privacy_migration_script(&without_config);

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
    for step in &removed {
        assert!(step.ok, "`joy {}` failed: {}", step.label, step.text);
    }
}

/// The half of D3.9 that git config used to answer, and must not: a
/// fresh clone or a second machine, WITH a git config that names a
/// registered member of this very project.
///
/// Before J11 that config was the deciding identity source, so this
/// machine acted as the person it named without anybody on it ever
/// saying so. Now nothing answers, every command that needs an identity
/// says so in the same sentence, and naming the member once settles it.
/// A read-only command keeps working throughout, because it needs no
/// member.
#[test]
fn a_git_config_alone_does_not_decide_who_acts() {
    let machine = Machine::new();
    machine.git_config_says("a@b.c");
    found_and_enrol(&machine);
    // The project file travels to the new machine; this device's session
    // and pin do not. The git config stays, and it names the founder.
    machine.forget_the_device_state();

    for (label, args) in [
        ("auth status", vec!["auth", "status"]),
        ("crypt status", vec!["crypt", "status"]),
        ("add an item", vec!["add", "task", "First thing"]),
        ("deauth", vec!["deauth"]),
    ] {
        let output = machine.joy(&args);
        assert!(
            !output.status.success(),
            "`joy {label}` answered from git config: {}",
            text(&output)
        );
        assert!(
            text(&output).contains("this project does not know who you are"),
            "`joy {label}` says what is missing: {}",
            text(&output)
        );
        assert!(
            text(&output).contains("joy auth --user <address>"),
            "`joy {label}` names the remedy: {}",
            text(&output)
        );
    }

    // Reading needs no member, and it never did.
    let ls = machine.joy(&["ls"]);
    assert!(ls.status.success(), "{}", text(&ls));

    // Naming the member once is the whole remedy, and it is the same one
    // the machine with no git config at all is given.
    let named = machine.joy(&["auth", "--user", "a@b.c", "--passphrase", PASSPHRASE]);
    assert!(named.status.success(), "{}", text(&named));
    let status = machine.joy(&["auth", "status"]);
    assert!(status.status.success(), "{}", text(&status));
    assert!(text(&status).contains("a@b.c"), "{}", text(&status));
    // ...and `joy auth status` says where the answer came from, because a
    // pin is this device's own state and nothing else on the machine
    // shows it.
    assert!(
        text(&status).contains("remembered on this device"),
        "the source of the answer is named: {}",
        text(&status)
    );
}

/// A delegation session answers WHO is acting, and nothing about what
/// they may do (D3.9): the rights question is the guard's, and the crypt
/// verbs that change who can read a zone ask it.
///
/// This is the shape J11 creates and therefore has to close: a machine
/// with no git config at all, where these verbs used to be stopped by
/// git2's "user.email is empty" and by nothing else. An AI session with
/// no manage capability, and one with every capability there is, are
/// both refused, and no passphrase is asked for on the way.
#[test]
fn a_delegation_session_cannot_change_who_reads_a_zone() {
    let machine = Machine::new();
    found_and_enrol(&machine);
    let item = machine.joy(&["add", "task", "First thing"]);
    assert!(item.status.success(), "{}", text(&item));
    let encrypted = machine.joy(&["crypt", "add", "LG-0001", "--passphrase", PASSPHRASE]);
    assert!(encrypted.status.success(), "{}", text(&encrypted));
    let session = machine.a_delegation_session("ai:claude@joy");

    for (label, args) in [
        (
            "crypt revoke",
            vec!["crypt", "revoke", "a@b.c", "--zone", "default"],
        ),
        (
            "crypt grant",
            vec![
                "crypt",
                "grant",
                "ai:claude@joy",
                "--zone",
                "default",
                "--passphrase",
                PASSPHRASE,
            ],
        ),
        ("crypt zone rm", vec!["crypt", "zone", "rm", "default"]),
    ] {
        let output = machine.joy_with_session(&args, Some(&session));
        assert!(
            !output.status.success(),
            "`joy {label}` let a delegation session change a zone's readers: {}",
            text(&output)
        );
        assert!(
            text(&output).contains("ai:claude@joy") && text(&output).contains("manage"),
            "`joy {label}` names the AI and the right it lacks: {}",
            text(&output)
        );
    }

    // The human whose access the AI tried to revoke still has it.
    let status = machine.joy(&["crypt", "status"]);
    assert!(status.status.success(), "{}", text(&status));
    let zone_still_readable = machine.joy(&["crypt", "ls"]);
    assert!(
        zone_still_readable.status.success(),
        "{}",
        text(&zone_still_readable)
    );
}

/// The order of D3.9, at its top: a delegation session outranks git
/// config. `joy deauth` under an AI session ends THAT session, and the
/// operator whose address the git config carries keeps theirs.
#[test]
fn a_delegation_session_outranks_the_git_config() {
    let machine = Machine::new();
    machine.git_config_says("a@b.c");
    found_and_enrol(&machine);

    let session_env = machine.a_delegation_session("ai:claude@joy");

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
    // Another machine, or a fresh clone: the project file travels, this
    // device's session and pin do not, and this one has no git config
    // either. That is the only state in which nothing can answer.
    machine.forget_the_device_state();
    machine.forget_the_git_config();

    let refused = machine.joy(&["auth", "--passphrase", PASSPHRASE]);
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

    // A command that takes no `--user` of its own says the same thing,
    // and the remedy it names is one it can actually be given.
    let crypt = machine.joy(&["crypt", "status"]);
    assert!(!crypt.status.success(), "{}", text(&crypt));
    assert!(
        text(&crypt).contains("joy auth --user <address>"),
        "{}",
        text(&crypt)
    );

    let named = machine.joy(&["auth", "--user", "a@b.c", "--passphrase", PASSPHRASE]);
    assert!(named.status.success(), "{}", text(&named));

    // Authenticating pinned the member, so the next command needs neither
    // a name nor a git config, including the one that has no `--user`.
    let status = machine.joy(&["auth", "status"]);
    assert!(text(&status).contains("a@b.c"), "{}", text(&status));
    let crypt = machine.joy(&["crypt", "status"]);
    assert!(crypt.status.success(), "{}", text(&crypt));
}

/// Under a delegation session, every command that needs a PASSPHRASE acts
/// for the operator who delegated, never for the AI the session names: an
/// AI member has no `kdf_nonce` and can never have one, so resolving it
/// there would refuse the operator work they are entitled to do.
///
/// This is the shape the CLI had before J11, where these commands read
/// git config and therefore always found the human. It must hold on a
/// machine with no git config at all.
#[test]
fn a_delegation_session_acts_for_the_operator_where_a_passphrase_is_needed() {
    let machine = Machine::new();
    found_and_enrol(&machine);
    let item = machine.joy(&["add", "task", "First thing"]);
    assert!(item.status.success(), "{}", text(&item));
    let session = machine.a_delegation_session("ai:claude@joy");

    // The zone commands: the operator's passphrase opens the operator's
    // seed, under the AI's session.
    let crypt_add = machine.joy_with_session(
        &["crypt", "add", "LG-0001", "--passphrase", PASSPHRASE],
        Some(&session),
    );
    assert!(crypt_add.status.success(), "{}", text(&crypt_add));
    let crypt_status = machine.joy_with_session(&["crypt", "status"], Some(&session));
    assert!(crypt_status.status.success(), "{}", text(&crypt_status));

    // A member write is a different matter: the guard refuses an AI
    // manage action whatever the passphrase says, and it names the AI in
    // the refusal, because the AI is who is acting. Identity and rights
    // are two questions, and only the first one moved in J11.
    let member_add = machine.joy_with_session(
        &[
            "project",
            "member",
            "add",
            "b@c.d",
            "--passphrase",
            PASSPHRASE,
        ],
        Some(&session),
    );
    assert!(!member_add.status.success(), "{}", text(&member_add));
    assert!(
        text(&member_add).contains("ai:claude@joy"),
        "{}",
        text(&member_add)
    );

    // And the passphrase change, which names the operator in what it
    // prints, not the AI.
    let changed = machine.joy_with_session(
        &[
            "auth",
            "passphrase",
            "--passphrase",
            PASSPHRASE,
            "--new-passphrase",
            SECOND_PASSPHRASE,
        ],
        Some(&session),
    );
    assert!(changed.status.success(), "{}", text(&changed));
    assert!(
        text(&changed).contains("a@b.c") && !text(&changed).contains("ai:claude@joy"),
        "the operator is the one whose passphrase changed: {}",
        text(&changed)
    );
}

/// An anonymous project hands every command the operator's OPAQUE id,
/// because that is what the member map is keyed by and what the member
/// pin behind `resolve_identity` holds (D3.9, package J11). Two writes on
/// the token path were still keyed by ADDRESS, and an address matches no
/// opaque id, so both quietly wrote nothing:
///
///  - `joy auth token add` skipped the `ai_delegations` entry it had just
///    derived the token from, and printed the token anyway;
///  - redeeming that token minted a session with no delegating operator
///    in its claims, which the F2 check refuses on the AI's next command.
///
/// The result was an AI that could be registered, delegated and handed a
/// token, and still not act: the commands that failed all reported
/// success, and the refusal arrived one step later as a hint on stderr
/// while the item was quietly written by the HUMAN instead. This is the
/// whole path, in the mode that breaks it.
#[test]
fn an_ai_token_of_an_anonymous_project_redeems_and_the_ai_acts() {
    let machine = Machine::new();
    found_anonymously(&machine);

    // Register the AI, issue its token, redeem it. Every step asserts its
    // own success, so the one that breaks names itself.
    let session = machine.a_delegation_session("ai:claude@joy");

    // The delegation the token was issued from is written down, under the
    // operator's own member entry. This is the entry redemption looks for,
    // and the one that used to be silently skipped here.
    let project = std::fs::read_to_string(machine.root.join(".joy").join("project.yaml")).unwrap();
    assert!(
        project.contains("ai_delegations:") && project.contains("delegation_verifier:"),
        "the issued delegation is recorded in project.yaml: {project}"
    );

    // The AI acts. Not "a command succeeds": the item has to be written
    // BY the AI, for the operator behind it, or the fallback identity has
    // simply stood in for a session that was refused.
    let written = machine.joy_with_session(&["add", "task", "Work of an AI"], Some(&session));
    assert!(written.status.success(), "{}", text(&written));
    assert!(
        !text(&written).contains("names no delegating operator"),
        "the session was refused: {}",
        text(&written)
    );
    let actor = the_actor_of_the_only_item(&machine);
    assert!(
        actor.starts_with("ai:claude@joy delegated-by:m-"),
        "the AI acts for the operator, by opaque id: {actor}"
    );

    // And the point of the mode is kept: naming the operator at rest
    // wrote no address anywhere under .joy/.
    assert!(
        !joy_dir_mentions(&machine, FOUNDER),
        "the founder's address must not reach a project file"
    );
}

/// A delegation token names its operator by their at-rest member key, in
/// both spellings the issuing command accepts.
///
/// `joy auth token add` takes the operator from `identity::acting_member`,
/// which hands back whatever it was given: the member this device pinned,
/// which in an anonymous project is the opaque id, or the raw string of a
/// `--user` flag, which is an address a person typed. The token used to
/// carry that string as its `delegated_by` claim, so the SAME operator of
/// the SAME project was written into the token two different ways, and
/// one of them was a cleartext address inside a credential that is then
/// pasted into chats, CI variables and agent configuration. The claim is
/// resolved once, at issuance, so neither end has to accept two forms and
/// the address never leaves the project.
#[test]
fn a_token_names_its_operator_by_key_however_the_operator_was_named() {
    let machine = Machine::new();
    found_anonymously(&machine);

    let add = machine.joy(&[
        "project",
        "member",
        "add",
        "ai:claude@joy",
        "--capabilities",
        "all",
        "--passphrase",
        PASSPHRASE,
    ]);
    assert!(add.status.success(), "{}", text(&add));

    // Named by address, which is the spelling that used to reach the
    // claim unresolved.
    let issued = machine.joy(&[
        "auth",
        "token",
        "add",
        "ai:claude@joy",
        "--user",
        FOUNDER,
        "--passphrase",
        PASSPHRASE,
        "--json",
    ]);
    assert!(issued.status.success(), "{}", text(&issued));
    let token = json_string(&text(&issued), "token");

    let claims = joy_core::auth::token::decode_token(&token)
        .expect("the token decodes")
        .claims;
    assert!(
        claims.delegated_by.starts_with("m-"),
        "the operator is claimed by opaque id: {}",
        claims.delegated_by
    );
    assert!(
        !token.contains(FOUNDER) && !claims.delegated_by.contains(FOUNDER),
        "no address rides in the token: {token}"
    );

    // And the token still works: the key form is what redemption and the
    // F2 check read, and the AI acts for the operator behind it.
    let redeemed = machine.joy(&["auth", "--token", &token, "--json"]);
    assert!(redeemed.status.success(), "{}", text(&redeemed));
    let session = json_string(&text(&redeemed), "session_env");
    let written = machine.joy_with_session(&["add", "task", "Work of an AI"], Some(&session));
    assert!(written.status.success(), "{}", text(&written));
    let actor = the_actor_of_the_only_item(&machine);
    assert_eq!(
        actor,
        format!("ai:claude@joy delegated-by:{}", claims.delegated_by),
        "the item names the operator by the very id the token claimed"
    );
    assert!(
        !joy_dir_mentions(&machine, FOUNDER),
        "the founder's address must not reach a project file"
    );
}

/// D3.9 promises a person that naming themselves once settles it: this
/// device remembers the member, and every later command knows them. In an
/// ANONYMOUS project what the device remembers is the opaque `m-<hex>`
/// id, because that is the member map's key (ADR-042), and three things
/// on the login path still wanted an address where the pin hands them an
/// id:
///
///  - the attestation check, which compares against the identifier the
///    attestation SIGNED, and an attestation never signs an opaque id;
///  - the re-lock of files left unlocked, which looked its member up by
///    address and so found nobody and re-locked nothing;
///  - the line the person reads, which printed the opaque id at them.
///
/// The first one locked every returning member of a multi-member
/// anonymous project out of their own project, with a message saying
/// their entry looked tampered with, until they typed `--user <address>`
/// again. That is the opposite of what the pin is for.
#[test]
fn an_anonymous_project_knows_its_members_from_the_pin_alone() {
    let machine = Machine::new();
    found_and_enrol(&machine);
    let second = machine.a_second_enrolled_member("b@c.d");
    assert!(second.status.success(), "{}", text(&second));

    // The second member's enrolment reverse-attested the founder, so
    // from here on both members carry an attestation over their address.
    let anonymous = machine.joy(&[
        "project",
        "set",
        "privacy",
        "anonymous",
        "--passphrase",
        SECOND_PASSPHRASE,
    ]);
    assert!(anonymous.status.success(), "{}", text(&anonymous));

    // b@c.d is the member this device pinned, and is the one the plain
    // `joy auth` speaks for: no address is typed anywhere below.
    let returning = machine.joy(&["auth", "--passphrase", SECOND_PASSPHRASE]);
    assert!(
        returning.status.success(),
        "a pinned member of an anonymous project authenticates: {}",
        text(&returning)
    );
    // And is told who they are in words, not as the project's own id.
    assert!(
        text(&returning).contains("Authenticated as b@c.d"),
        "{}",
        text(&returning)
    );
    assert!(
        !text(&returning).contains("Authenticated as m-"),
        "an opaque id is never what a person is shown (ADR-042): {}",
        text(&returning)
    );

    // The founder, named once, is then equally known from the pin alone.
    let named = machine.joy(&["auth", "--user", "a@b.c", "--passphrase", PASSPHRASE]);
    assert!(named.status.success(), "{}", text(&named));
    let again = machine.joy(&["auth", "--passphrase", PASSPHRASE]);
    assert!(again.status.success(), "{}", text(&again));
    assert!(
        text(&again).contains("Authenticated as a@b.c"),
        "{}",
        text(&again)
    );
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
