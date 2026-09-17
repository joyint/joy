// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! joy on a machine with no git binary (JOY-01FD-ED, design D3.2).
//!
//! The operator's reason is mobile: the app must work where there is no
//! git to spawn. A layer that is "mostly git2" fails there on the one
//! path nobody tested, so this test takes the binary away and walks the
//! whole level 1 journey: init, an item write with the commit joy makes
//! for it, a chat sent and pushed to a real remote, and a release
//! recorded with its annotated tag.
//!
//! PATH is stripped of every directory that carries a `git`, and the
//! test refuses to pass if one is still reachable: a green run against a
//! machine that still has git would prove nothing at all.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Output;

const PASSPHRASE: &str = "correct horse battery staple";

/// Every PATH entry that does NOT carry an executable called `git`.
fn path_without_git() -> OsString {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let kept: Vec<PathBuf> = std::env::split_paths(&path)
        .filter(|dir: &PathBuf| !dir.join("git").is_file() && !dir.join("git.exe").is_file())
        .collect();
    std::env::join_paths(kept).expect("a PATH without git")
}

/// Whether `git` can still be started from `path`. The test's own
/// premise, checked rather than assumed.
fn git_is_reachable(path: &OsString) -> bool {
    std::env::split_paths(path)
        .any(|dir| dir.join("git").is_file() || dir.join("git.exe").is_file())
}

struct Machine {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    path: OsString,
}

impl Machine {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        Self {
            _dir: dir,
            root,
            home,
            path: path_without_git(),
        }
    }

    /// The identity this checkout carries, written into `.git/config`
    /// the way a person's `git config user.email` would have. A FILE,
    /// not a process: what this test takes away is the git binary, not
    /// the config git and libgit2 both read.
    fn set_git_identity(&self, email: &str) {
        let repo = git2::Repository::open(&self.root).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Scotty").unwrap();
        config.set_str("user.email", email).unwrap();
    }

    /// Run `joy` with no git anywhere on PATH.
    fn joy(&self, args: &[&str]) -> Output {
        joy_process::command(env!("CARGO_BIN_EXE_joy"))
            .args(args)
            .current_dir(&self.root)
            .env("PATH", &self.path)
            .env("HOME", &self.home)
            .env("XDG_STATE_HOME", self.home.join(".state"))
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("JOY_SESSION")
            .output()
            .expect("joy runs")
    }

    /// The same, as an agent: under a delegation session, with nothing
    /// on stdin to read (D3.8).
    ///
    /// The session has to be a live one. A `JOY_SESSION` that merely
    /// exists leaves the host as interactive as it was
    /// (host.rs:119-133), so a value joy cannot load would prove nothing
    /// about the delegated host this case is about.
    fn joy_as_agent(&self, session: &str, args: &[&str]) -> Output {
        joy_process::command(env!("CARGO_BIN_EXE_joy"))
            .args(args)
            .current_dir(&self.root)
            .env("PATH", &self.path)
            .env("HOME", &self.home)
            .env("XDG_STATE_HOME", self.home.join(".state"))
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("JOY_SESSION", session)
            .stdin(std::process::Stdio::null())
            .output()
            .expect("joy runs")
    }

    /// A delegation session, minted the way an agent really gets one: an
    /// AI member, a delegation token, and the redemption that prints the
    /// handle. Every step of it runs on this machine without git, so the
    /// enrolment an agent needs is part of what this file proves.
    fn a_live_session(&self) -> String {
        let member = "ai:claude@joy";
        let added = self.joy(&[
            "project",
            "member",
            "add",
            member,
            "--passphrase",
            PASSPHRASE,
        ]);
        assert!(added.status.success(), "{}", text(&added));
        let minted = self.joy(&["auth", "token", "add", member, "--passphrase", PASSPHRASE]);
        assert!(minted.status.success(), "{}", text(&minted));
        // The token is the quoted line among the instructions.
        let token = text(&minted)
            .lines()
            .find_map(|line| {
                let quoted = line.trim().strip_prefix('"')?;
                Some(quoted.strip_suffix('"')?.to_string())
            })
            .expect("the token is printed");
        let redeemed = self.joy(&["auth", "--token", &token]);
        assert!(redeemed.status.success(), "{}", text(&redeemed));
        text(&redeemed)
            .lines()
            .find_map(|line| line.strip_prefix("export JOY_SESSION="))
            .expect("the redemption prints the handle")
            .trim()
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

fn head_summary(root: &Path) -> String {
    let repo = git2::Repository::open(root).unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    head.summary()
        .ok()
        .flatten()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn joy_works_end_to_end_on_a_machine_without_git() {
    let machine = Machine::new();
    assert!(
        !git_is_reachable(&machine.path),
        "this test only proves something when git really is gone from PATH"
    );

    // 1. init: the repository itself is created by libgit2.
    let init = machine.joy(&["init", "--name", "NoGit", "--user", "scotty@example.com"]);
    assert!(init.status.success(), "{}", text(&init));
    assert!(machine.root.join(".git").exists(), "{}", text(&init));
    assert!(machine.root.join(".joy/project.yaml").is_file());

    // Chats are always sealed, so the project needs an identity before
    // one can be written (JOY-021D-F4).
    machine.set_git_identity("scotty@example.com");
    let auth = machine.joy(&["auth", "init", "--passphrase", PASSPHRASE]);
    assert!(auth.status.success(), "{}", text(&auth));

    // joy commits its own writes from here on.
    std::fs::write(
        machine.root.join(".joy/config.yaml"),
        "workflow:\n  auto-git: commit\n",
    )
    .unwrap();

    // A remote a push can really reach, and no network anywhere near it.
    let forge = machine._dir.path().join("forge.git");
    git2::Repository::init_bare(&forge).unwrap();
    let repo = git2::Repository::open(&machine.root).unwrap();
    repo.remote("origin", forge.to_str().unwrap()).unwrap();

    // 2. an item write, and the commit joy makes for it.
    let add = machine.joy(&["add", "task", "First thing"]);
    assert!(add.status.success(), "{}", text(&add));
    assert!(
        head_summary(&machine.root).starts_with("joy: add"),
        "joy committed its own write: {}",
        head_summary(&machine.root)
    );

    // 3. a chat, sent and delivered to the forge.
    let send = machine.joy(&[
        "chat",
        "send",
        "general",
        "hello from nowhere",
        "--passphrase",
        PASSPHRASE,
    ]);
    assert!(send.status.success(), "{}", text(&send));
    assert!(text(&send).contains("message sent"), "{}", text(&send));
    let forge_repo = git2::Repository::open_bare(&forge).unwrap();
    let chats = forge_repo
        .refname_to_id("refs/joy/chats")
        .expect("the chat ref reached the forge without a git process");
    assert!(!chats.is_zero());

    // ...and reading it back over the same transport.
    let read = machine.joy(&["chat", "show", "general", "--passphrase", PASSPHRASE]);
    assert!(read.status.success(), "{}", text(&read));
    assert!(
        text(&read).contains("hello from nowhere"),
        "{}",
        text(&read)
    );

    // 4. a release: the commit and the annotated tag, both libgit2's.
    let record = machine.joy(&["release", "record", "patch"]);
    assert!(record.status.success(), "{}", text(&record));
    let tag = repo.revparse_single("v0.0.1").unwrap();
    let tag = tag.as_tag().expect("an annotated tag");
    assert_eq!(tag.tagger().unwrap().email().unwrap(), "scotty@example.com");

    // 5. and the tag push, over the same local remote.
    let publish = machine.joy(&["release", "publish", "--forge", "none"]);
    assert!(publish.status.success(), "{}", text(&publish));
    assert!(
        forge_repo.refname_to_id("refs/tags/v0.0.1").is_ok(),
        "the tag reached the forge: {}",
        text(&publish)
    );
}

/// D3.8: an agent under a delegation session is never asked anything,
/// and what it reads is a stable word. The remote here is a path that
/// does not exist, so the contact fails for certain; the run must still
/// end by itself and name the state rather than waiting for somebody to
/// type something.
#[test]
fn an_agent_under_a_delegation_is_never_asked_and_reads_a_stable_state_word() {
    let machine = Machine::new();
    assert!(!git_is_reachable(&machine.path));

    let init = machine.joy(&["init", "--name", "Agent", "--user", "scotty@example.com"]);
    assert!(init.status.success(), "{}", text(&init));
    machine.set_git_identity("scotty@example.com");
    let auth = machine.joy(&["auth", "init", "--passphrase", PASSPHRASE]);
    assert!(auth.status.success(), "{}", text(&auth));
    let session = machine.a_live_session();

    let repo = git2::Repository::open(&machine.root).unwrap();
    let nowhere = machine._dir.path().join("no-such-forge.git");
    repo.remote("origin", nowhere.to_str().unwrap()).unwrap();

    // The chat is committed locally first, so a failed delivery is not
    // fatal and the command still succeeds. No passphrase either: the
    // session is who the agent is, which is what makes this the agent's
    // own path and not a person's.
    let send = machine.joy_as_agent(&session, &["--json", "chat", "send", "general", "hello"]);
    assert!(send.status.success(), "{}", text(&send));

    // One JSON object on stderr, carrying the state word. stdout stays
    // the command's own answer, because a second object there is a
    // corrupt answer.
    let answer = String::from_utf8_lossy(&send.stdout).to_string();
    assert!(
        !answer.contains("\"state\""),
        "the refusal never reaches stdout: {answer:?}"
    );
    let said = String::from_utf8_lossy(&send.stderr).to_string();
    let line = said
        .lines()
        .find(|line| line.starts_with('{'))
        .unwrap_or_else(|| panic!("a machine readable refusal on stderr: {said:?}"));
    let refusal: serde_json::Value = serde_json::from_str(line).unwrap();
    let state = refusal["state"].as_str().unwrap_or_default();
    assert!(
        [
            "needs_sign_in",
            "needs_org_approval",
            "needs_sso",
            "no_push_rights",
            "needs_host_trust",
            "scope_missing",
            "plugin_missing",
            "plugin_outdated",
            "tls_untrusted",
            "proxy_auth",
            "rate_limited",
            "offline",
            "denied",
            "error",
        ]
        .contains(&state),
        "the state is one of the classifier's words, got {state:?} in {line}"
    );
    assert!(
        !refusal["message"].as_str().unwrap_or_default().is_empty(),
        "the refusal carries its plain sentence: {line}"
    );
}
