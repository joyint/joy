// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! What the commits joy writes for itself carry (D4.5 and D3.4 of the
//! forge connection NG design, JOY-0297-1A), driven through the real
//! binary because both rules are only true if they hold on the paths the
//! commands actually take.
//!
//! The auth commands are the interesting ones: a host may still name the
//! member to them as a raw address (`--user`, the desktop's mask), so
//! they are the ones that would put a person's address into a commit of
//! an anonymous project if the signature gate decided by the shape of the
//! string it was handed instead of by the project.

use std::path::Path;
use std::process::Output;

const PASSPHRASE: &str = "correct horse battery staple";

/// Run `joy` in `root` with an isolated home, so no git identity of this
/// machine can answer for the person under test.
fn joy(root: &Path, home: &Path, args: &[&str]) -> Output {
    joy_process::command(env!("CARGO_BIN_EXE_joy"))
        .args(args)
        .current_dir(root)
        .env("HOME", home)
        .env("XDG_STATE_HOME", home.join(".state"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("JOY_SESSION")
        .output()
        .expect("joy runs")
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn machine() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    (dir, root, home)
}

/// joy commits its own writes from here on.
fn auto_git_commit(root: &Path) {
    std::fs::write(
        root.join(".joy/config.yaml"),
        "workflow:\n  auto-git: commit\n",
    )
    .unwrap();
}

/// The four signature fields of the commit at HEAD.
fn head_fields(root: &Path) -> [String; 4] {
    let repo = git2::Repository::open(root).unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    let author = head.author();
    let committer = head.committer();
    let fields = [
        author.name().unwrap().to_string(),
        author.email().unwrap().to_string(),
        committer.name().unwrap().to_string(),
        committer.email().unwrap().to_string(),
    ];
    fields
}

/// D4.5 through a command that holds the person's ADDRESS: `joy auth
/// passphrase` names the member by address, and in an anonymous project
/// the commit it writes must still carry the opaque id in all four
/// signature fields. Deciding by the shape of that string signed the
/// address instead, which is how ADR-042 was undone by a git2 commit.
#[test]
fn an_auth_command_in_an_anonymous_project_commits_under_the_opaque_id() {
    let (_dir, root, home) = machine();

    let init = joy(
        &root,
        &home,
        &[
            "init",
            "--name",
            "Anon",
            "--user",
            "scotty@example.com",
            "--anonymous",
            "--passphrase",
            PASSPHRASE,
        ],
    );
    assert!(init.status.success(), "{}", text(&init));

    // This checkout HAS a git identity, and it is the one `git commit`
    // would have signed with.
    let repo = git2::Repository::open(&root).unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "Scotty Real").unwrap();
    config.set_str("user.email", "scotty@example.com").unwrap();
    auto_git_commit(&root);

    let changed = joy(
        &root,
        &home,
        &[
            "auth",
            "passphrase",
            "--passphrase",
            PASSPHRASE,
            "--new-passphrase",
            "another horse battery staple entirely",
        ],
    );
    assert!(changed.status.success(), "{}", text(&changed));

    let fields = head_fields(&root);
    for field in &fields {
        assert!(
            field.starts_with("m-"),
            "every signature field is the opaque member id, got {fields:?}"
        );
        assert!(!field.contains('@'), "no address in {fields:?}");
        assert!(!field.contains("Scotty"), "no person's name in {fields:?}");
    }

    // The message is part of the same committed object, and the
    // `Co-Authored-By` trailer is written from the same acting member.
    let repo = git2::Repository::open(&root).unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    let message = head.message().unwrap_or_default().to_string();
    assert!(message.contains("Co-Authored-By: m-"), "{message}");
    assert!(!message.contains("scotty@example.com"), "{message}");
}

/// D3.4: a commit joy writes carries the paths joy wrote and nothing
/// else. The person's half staged work stays theirs, and it does not
/// defeat the "nothing to commit" answer either.
#[test]
fn a_joy_commit_leaves_the_persons_staged_work_where_it_was() {
    let (_dir, root, home) = machine();

    let init = joy(
        &root,
        &home,
        &["init", "--name", "Scoped", "--user", "scotty@example.com"],
    );
    assert!(init.status.success(), "{}", text(&init));
    auto_git_commit(&root);

    // The person is in the middle of something and has staged part of it.
    std::fs::write(root.join("src.rs"), "half finished\n").unwrap();
    let repo = git2::Repository::open(&root).unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("src.rs")).unwrap();
    index.write().unwrap();

    let add = joy(&root, &home, &["add", "task", "First thing"]);
    assert!(add.status.success(), "{}", text(&add));

    let head = repo.head().unwrap().peel_to_commit().unwrap();
    let tree = head.tree().unwrap();
    let summary = head
        .summary()
        .ok()
        .flatten()
        .unwrap_or_default()
        .to_string();
    assert!(
        summary.starts_with("joy: add"),
        "joy wrote the commit at HEAD: {summary}"
    );
    assert!(
        tree.get_path(Path::new("src.rs")).is_err(),
        "the person's staged file is not joy's to commit"
    );
    assert!(
        tree.get_path(Path::new(".joy/project.yaml")).is_ok(),
        "joy's own files are in the commit"
    );

    // ...and it is still staged, where the person left it.
    let index = repo.index().unwrap();
    assert!(index.get_path(Path::new("src.rs"), 0).is_some());
}
