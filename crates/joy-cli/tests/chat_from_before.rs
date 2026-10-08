// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! A chat that an earlier release wrote still opens, and the AI member
//! in it is ONE member, known by its name (JI-019D-46).
//!
//! `tests/fixtures/chat-from-before/project.bundle` is a project recorded
//! with joy 0.22: an AI member written in the legacy form, a person's
//! delegation to it, and on `refs/joy/chats` a sealed team chat with that
//! AI member as participant, a line of the person, an answer of the AI
//! member and its session. It stays as it was recorded: this is what the
//! repositories out there hold.
//!
//! On integration one line got two answers (2026-10-08). The release
//! after 0.22 read such a chat as it stood, so the AI member was in it
//! under the legacy form, a mention added it again under its name, and
//! the client asks a turn of every AI participant. The same release
//! rotated the chat's key at every write, because the member covered
//! under the legacy form counted as someone who had left. No test opened
//! a chat written by an earlier release; every one of them began with a
//! chat it had just written itself. This one does, through the real
//! binary, and it fails on that release at every step below.

joy_test_env::isolate!();

use std::path::{Path, PathBuf};
use std::process::Output;

const FOUNDER: &str = "founder@example.com";
const PASSPHRASE: &str = "correct horse battery staple";
const AI: &str = "vibe";

struct Project {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
}

fn bundle() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/chat-from-before/project.bundle")
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

impl Project {
    /// A clone of the recorded project, chats included, on a machine
    /// that knows nobody.
    fn from_before() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let project = Project {
            _dir: dir,
            root,
            home,
        };
        let bundle = bundle();
        let bundle = bundle.to_str().unwrap();
        let root = project.root.to_str().unwrap().to_string();
        project.git_at(project.home.as_path(), &["clone", "--quiet", bundle, &root]);
        project.git(&["fetch", "--quiet", "origin", "+refs/joy/*:refs/joy/*"]);
        // the bundle is no forge: nothing is fetched from it again
        project.git(&["remote", "remove", "origin"]);
        project.git(&["config", "user.email", FOUNDER]);
        project.git(&["config", "user.name", "Founder"]);
        project
    }

    fn git(&self, args: &[&str]) -> String {
        self.git_at(&self.root, args)
    }

    fn git_at(&self, dir: &Path, args: &[&str]) -> String {
        let out = joy_test_env::command("git")
            .args(args)
            .current_dir(dir)
            .env("HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?}: {}", text(&out));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn joy(&self, args: &[&str]) -> String {
        let out = joy_test_env::command(env!("CARGO_BIN_EXE_joy"))
            .args(args)
            .args(["--passphrase", PASSPHRASE])
            .current_dir(&self.root)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_STATE_HOME", self.home.join(".state"))
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("joy runs");
        assert!(out.status.success(), "joy {args:?}: {}", text(&out));
        text(&out)
    }

    /// How many key slots the chats hold: one per member a chat is
    /// sealed for, and more only when a key was made anew.
    fn key_slots(&self) -> usize {
        self.git(&["ls-tree", "-r", "--name-only", "refs/joy/chats"])
            .lines()
            .filter(|path| path.contains("/keys/"))
            .count()
    }

    /// The id people address the recorded chat by.
    fn chat(&self) -> String {
        let listed = self.joy(&["chat", "ls"]);
        listed
            .split_whitespace()
            .find(|word| word.starts_with("CB-CHAT-"))
            .unwrap_or_else(|| panic!("the recorded chat is listed: {listed}"))
            .to_string()
    }
}

#[test]
fn a_chat_written_by_an_earlier_release_holds_its_ai_member_once_under_its_name() {
    let legacy = joy_core::migrations::ai_member_name::legacy_form(AI);
    let project = Project::from_before();
    // what was recorded: the person and the AI member, a slot for each
    let recorded_slots = project.key_slots();
    assert_eq!(recorded_slots, 2);
    assert!(
        std::fs::read_to_string(project.root.join(".joy/project.yaml"))
            .unwrap()
            .contains(&legacy),
        "the recording still holds the AI member in the legacy form"
    );

    // It opens, with everything said in it, and the AI member reads as
    // its name.
    let chat = project.chat();
    let shown = project.joy(&["chat", "show", &chat]);
    assert!(shown.contains("2 participant(s)"), "{shown}");
    assert!(shown.contains("a first line from before"), "{shown}");
    assert!(
        shown.contains(&format!(
            "{AI} (delegated by {FOUNDER})  an answer from before"
        )),
        "{shown}"
    );
    assert!(!shown.contains(&legacy), "{shown}");
    assert_eq!(
        project.key_slots(),
        recorded_slots,
        "opening it makes no key"
    );

    // A line written today lands in it, and nobody left, so the chat's
    // key stays the one it has.
    project.joy(&["chat", "send", &chat, "a line from today"]);
    let shown = project.joy(&["chat", "show", &chat]);
    assert!(shown.contains("a line from today"), "{shown}");
    assert!(shown.contains("an answer from before"), "{shown}");
    assert_eq!(project.key_slots(), recorded_slots, "a write makes no key");

    // Addressing the AI member takes it into a chat it is not in yet.
    // It is in this one: by its name and typed in the legacy form alike
    // it stays the ONE member it is. Two entries were two turns, and two
    // answers to one line.
    project.joy(&["chat", "add", &chat, AI]);
    project.joy(&["chat", "add", &chat, &legacy]);
    let info = project.joy(&["chat", "info", &chat]);
    assert!(info.contains("2 participant(s)"), "{info}");
    let ai_rows = info
        .lines()
        .filter(|line| line.split_whitespace().next() == Some(AI))
        .count();
    assert_eq!(ai_rows, 1, "{info}");
    assert!(!info.contains(&legacy), "{info}");
    assert_eq!(project.key_slots(), recorded_slots, "and still no new key");

    // The person was here with their key, so the project is brought over,
    // and nothing it keeps says the legacy form any more.
    for kept in files_under(&project.root.join(".joy")) {
        let body = std::fs::read_to_string(&kept).unwrap_or_default();
        assert!(!body.contains(&legacy), "{} still says it", kept.display());
    }
}

/// Every file under `dir`.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(files_under(&path));
        } else {
            out.push(path);
        }
    }
    out
}
