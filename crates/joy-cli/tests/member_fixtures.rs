// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! A project from before the member files still opens and still works
//! (JOY-02C2-F3).
//!
//! `tests/fixtures/members/` holds two projects in the `.joy` format of
//! joy 0.22, one open and one anonymous, each with a founder, a second
//! person, an AI member with its delegation, and a token issued for it.
//! They stay in that format: this is what the repositories out there
//! look like, and whatever a later version does to the format has to
//! leave these cases true.
//!
//! Every case brings the fixture up to date the way one of the three
//! hosts does it, then asks the same questions through the real binary:
//! can the people still sign in, does the issued token still redeem, may
//! the AI still do what it was given and nothing more.

joy_test_env::isolate!();

use std::path::{Path, PathBuf};
use std::process::Output;

use joy_core::update::Reach;

const FOUNDER: &str = "founder@example.com";
const SECOND: &str = "second@example.com";
const FOUNDER_PASS: &str = "correct horse battery staple extra words";
const SECOND_PASS: &str = "alpha bravo charlie delta echo foxtrot";

/// How a host brings a project up to date when it opens one.
#[derive(Clone, Copy, Debug)]
enum Host {
    /// The platform at a fetch: the data reconciles only.
    Platform,
    /// The desktop at open: everything a checkout gets.
    Desktop,
    /// A person at a terminal.
    Cli,
}

struct Project {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    token: String,
    anonymous: bool,
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/members")
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

impl Project {
    /// A fresh clone of the fixture `name` on a machine that knows nobody.
    fn clone_of(name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        copy_tree(&fixtures().join(name).join("dot-joy"), &root.join(".joy"));
        git2::Repository::init(&root).unwrap();
        let token = std::fs::read_to_string(fixtures().join(name).join("ai-token"))
            .unwrap()
            .trim()
            .to_string();
        Project {
            _dir: dir,
            root,
            home,
            token,
            anonymous: name == "anonymous",
        }
    }

    fn joy(&self, args: &[&str]) -> Output {
        self.joy_with(args, None)
    }

    fn joy_with(&self, args: &[&str], session: Option<&str>) -> Output {
        let mut command = joy_test_env::command(env!("CARGO_BIN_EXE_joy"));
        command
            .args(args)
            .current_dir(&self.root)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_STATE_HOME", self.home.join(".state"))
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1");
        if let Some(session) = session {
            command.env("JOY_SESSION", session);
        }
        command.output().expect("joy runs")
    }

    fn open_as(&self, host: Host) {
        match host {
            Host::Platform => {
                joy_chat_store::update::sync(&self.root, Reach::Data);
            }
            Host::Desktop => {
                joy_chat_store::update::sync(&self.root, Reach::Checkout);
            }
            Host::Cli => {
                let out = self.joy(&["update"]);
                assert!(out.status.success(), "joy update: {}", text(&out));
            }
        }
    }

    /// Redeem the issued token and hand back the AI's session.
    fn ai_session(&self) -> String {
        let out = self.joy(&["auth", "--token", &self.token, "--json"]);
        assert!(
            out.status.success(),
            "the issued token redeems: {}",
            text(&out)
        );
        let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        // Who delegated: the founder by address in an open project. An
        // anonymous one names nobody's address to an AI.
        let delegated_by = json["data"]["delegated_by"].as_str().unwrap_or_default();
        if self.anonymous {
            assert!(
                !delegated_by.is_empty() && !delegated_by.contains('@'),
                "{json}"
            );
        } else {
            assert_eq!(delegated_by, FOUNDER, "{json}");
        }
        json["data"]["session_env"].as_str().unwrap().to_string()
    }
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
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

/// The item from before, as text.
fn item_from_before(root: &Path) -> PathBuf {
    files_under(&root.join(".joy/items"))
        .into_iter()
        .find(|path| {
            std::fs::read_to_string(path)
                .unwrap_or_default()
                .contains("A task from before the member files")
        })
        .expect("the item from before")
}

/// What an AI member left in a project of that time: its legacy form in
/// the item it last touched and in a line of the log. Returns the two
/// files with what they hold now.
fn written_by_the_ai_back_then(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let older = joy_core::migrations::ai_member_name::legacy_form("claude");
    let item = item_from_before(root);
    let body = std::fs::read_to_string(&item).unwrap();
    let person = body
        .lines()
        .find_map(|line| line.strip_prefix("updated_by: "))
        .expect("who updated it")
        .to_string();
    let by_the_ai = format!("{older} delegated-by:{person}");
    std::fs::write(
        &item,
        body.replace(
            &format!("updated_by: {person}"),
            &format!("updated_by: {by_the_ai}"),
        ),
    )
    .unwrap();
    let log = files_under(&root.join(".joy/logs"))
        .into_iter()
        .next()
        .expect("the log from before");
    let mut lines = std::fs::read_to_string(&log).unwrap();
    lines.push_str(&format!(
        "2026-10-07T20:26:00.000Z FX-0001-30 item.updated [{by_the_ai}]\n"
    ));
    std::fs::write(&log, lines).unwrap();
    [item, log]
        .into_iter()
        .map(|path| {
            let bytes = std::fs::read(&path).unwrap();
            assert!(
                String::from_utf8_lossy(&bytes).contains(&older),
                "{} says the legacy form",
                path.display()
            );
            (path, bytes)
        })
        .collect()
}

/// The questions, the same for every fixture and every host.
fn still_works(fixture: &str, host: Host) {
    let project = Project::clone_of(fixture);
    let records = written_by_the_ai_back_then(&project.root);
    project.open_as(host);
    // Opening it changes nothing about where its members are kept: no
    // host that merely opens a project holds anybody's key.
    assert!(
        !project.root.join(".joy/members").exists(),
        "{fixture} opened as {host:?}: still as it was"
    );
    let ctx = format!("{fixture} opened as {host:?}");

    // `joy update` with nobody signed in whose key could sign says what
    // it waits for, and leaves the project as it is.
    let waits = project.joy(&["update", "--no-binary"]);
    assert!(waits.status.success(), "{ctx}: {}", text(&waits));
    assert!(
        text(&waits).contains("sign in with joy auth, then run joy update"),
        "{ctx}: {}",
        text(&waits)
    );
    assert!(!project.root.join(".joy/members").exists(), "{ctx}");

    // The second person signs in with their passphrase, may write an
    // item, and may not manage the project.
    let auth = project.joy(&["auth", "--user", SECOND, "--passphrase", SECOND_PASS]);
    assert!(
        auth.status.success(),
        "{ctx}: second signs in: {}",
        text(&auth)
    );
    let add = project.joy(&["add", "task", "Written by the second person"]);
    assert!(add.status.success(), "{ctx}: second writes: {}", text(&add));
    let manage = project.joy(&["project", "set", "description", "not theirs to say"]);
    assert!(
        !manage.status.success(),
        "{ctx}: second holds no manage: {}",
        text(&manage)
    );

    // The founder signs in and manages.
    let auth = project.joy(&["auth", "--user", FOUNDER, "--passphrase", FOUNDER_PASS]);
    assert!(
        auth.status.success(),
        "{ctx}: founder signs in: {}",
        text(&auth)
    );
    let manage = project.joy(&["project", "set", "description", "said by the founder"]);
    assert!(
        manage.status.success(),
        "{ctx}: founder manages: {}",
        text(&manage)
    );

    // The token issued before still redeems; the AI writes an item under
    // its delegation and may not manage the project.
    let session = project.ai_session();
    let add = project.joy_with(
        &["add", "task", "Written by the AI", "--json"],
        Some(&session),
    );
    assert!(add.status.success(), "{ctx}: the AI writes: {}", text(&add));
    let manage = project.joy_with(
        &["project", "set", "description", "not the AI's to say"],
        Some(&session),
    );
    assert!(
        !manage.status.success(),
        "{ctx}: the AI never manages: {}",
        text(&manage)
    );

    // Two people signed in, wrote and managed, an AI redeemed its token
    // and wrote: the project still keeps its members where it kept them
    // (JOY-02CC-C1). Bringing it over changes what an older joy can
    // read, so no sign-in and no other command does it on the side.
    assert!(
        !project.root.join(".joy/members").exists(),
        "{ctx}: nobody asked for an update, so nothing was brought over"
    );
    // The AI member reads as its name all the same, from the files as
    // they stand.
    let shown = project.joy(&["show", "FX-0001-30", "--json"]);
    let older = joy_core::migrations::ai_member_name::legacy_form("claude");
    assert!(shown.status.success(), "{ctx}: {}", text(&shown));
    assert!(text(&shown).contains("claude"), "{ctx}: {}", text(&shown));
    assert!(!text(&shown).contains(&older), "{ctx}: {}", text(&shown));

    // The founder signs in and asks for the update: now the project is
    // brought over (JI-019D-46): one file per member, project.yaml lists
    // them, and the AI member is known by its name.
    let auth = project.joy(&["auth", "--user", FOUNDER, "--passphrase", FOUNDER_PASS]);
    assert!(auth.status.success(), "{ctx}: {}", text(&auth));
    let update = project.joy(&["update", "--no-binary"]);
    assert!(update.status.success(), "{ctx}: {}", text(&update));
    assert!(
        text(&update).contains("moved to their own files"),
        "{ctx}: {}",
        text(&update)
    );
    let members = joy_test_env::project_text(&project.root);
    let files = std::fs::read_dir(project.root.join(".joy/members"))
        .map(|dir| dir.count())
        .unwrap_or(0);
    assert_eq!(files, 3, "{ctx}: three member files");
    assert!(members.contains("name: claude"), "{ctx}: {members}");
    // project.yaml and the member files do not say the legacy form any
    // more...
    assert!(!members.contains(&older), "{ctx}: {members}");
    // ...and nothing else was touched (JOY-02CC-C1): the item and the
    // log an AI member wrote back then are byte for byte what they
    // were. In a real project that is every item, job, milestone,
    // release and log it ever touched (117 files in one of ours).
    for (path, before) in &records {
        assert_eq!(
            &std::fs::read(path).unwrap(),
            before,
            "{ctx}: {} was rewritten",
            path.display()
        );
    }
    // They read with the name, as before.
    let shown = project.joy(&["show", "FX-0001-30", "--json"]);
    assert!(text(&shown).contains("claude"), "{ctx}: {}", text(&shown));
    assert!(!text(&shown).contains(&older), "{ctx}: {}", text(&shown));
    let log = project.joy(&["log"]);
    assert!(log.status.success(), "{ctx}: {}", text(&log));
    assert!(!text(&log).contains(&older), "{ctx}: {}", text(&log));
    assert!(!members.contains("attestation:"), "{ctx}: {members}");
    assert!(members.contains("granted:"), "{ctx}: {members}");
    // What the AI wrote before the project was brought over was written
    // under its name already: nothing joy writes carries the legacy form.
    let item = std::fs::read_dir(project.root.join(".joy/items"))
        .unwrap()
        .flatten()
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
        .find(|body| body.contains("Written by the AI"))
        .expect("the AI's item");
    assert!(
        item.contains("created_by: claude delegated-by:"),
        "{ctx}: the AI writes under its name: {item}"
    );

    // The delegation from before is still the founder's to use: a new
    // token for the AI member is issued without a new delegation, under
    // its name, and redeems. The key behind it was derived when the
    // member was written by its legacy form, and it is the same key now.
    let issued = project.joy(&[
        "auth",
        "token",
        "add",
        "claude",
        // redeeming made the AI the one signed in on this terminal, so
        // the founder says who they are
        "--user",
        FOUNDER,
        "--passphrase",
        FOUNDER_PASS,
    ]);
    assert!(
        issued.status.success(),
        "{ctx}: a new token for claude: {}",
        text(&issued)
    );
    let token = String::from_utf8_lossy(&issued.stdout)
        .split_whitespace()
        .find(|word| word.trim_matches('"').starts_with("joy_t_"))
        .map(|word| word.trim_matches('"').to_string())
        .unwrap_or_else(|| panic!("{ctx}: a token in the answer: {}", text(&issued)));
    let redeemed = project.joy(&["auth", "--token", &token, "--json"]);
    assert!(
        redeemed.status.success(),
        "{ctx}: the new token redeems: {}",
        text(&redeemed)
    );
    let json: serde_json::Value = serde_json::from_slice(&redeemed.stdout).unwrap();
    let session = json["data"]["session_env"].as_str().unwrap().to_string();
    let add = project.joy_with(
        &["add", "task", "Written by the AI under its new token"],
        Some(&session),
    );
    assert!(
        add.status.success(),
        "{ctx}: the AI writes under its new token: {}",
        text(&add)
    );

    // And the item from before is still there to be read.
    let ls = project.joy(&["ls"]);
    assert!(
        text(&ls).contains("A task from before the member files"),
        "{ctx}: {}",
        text(&ls)
    );
}

#[test]
fn an_open_project_still_works_after_the_platform_opened_it() {
    still_works("open", Host::Platform);
}

#[test]
fn an_open_project_still_works_after_the_desktop_opened_it() {
    still_works("open", Host::Desktop);
}

#[test]
fn an_open_project_still_works_after_joy_update() {
    still_works("open", Host::Cli);
}

#[test]
fn an_anonymous_project_still_works_after_the_platform_opened_it() {
    still_works("anonymous", Host::Platform);
}

#[test]
fn an_anonymous_project_still_works_after_the_desktop_opened_it() {
    still_works("anonymous", Host::Desktop);
}

#[test]
fn an_anonymous_project_still_works_after_joy_update() {
    still_works("anonymous", Host::Cli);
}
