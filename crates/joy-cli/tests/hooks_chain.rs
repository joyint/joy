// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! joy owns `core.hooksPath` and chains (design D3.5).
//!
//! `core.hooksPath` REPLACES the hook location entirely, so the two
//! choices are "joy's check runs for nobody" and "joy runs what was
//! there before". This proves the second one on a repository that has
//! husky: joy's own commit-msg check runs FIRST, and husky's hook runs
//! after it.
//!
//! The hooks are bash, so the run needs a shell. A machine without one
//! has no hooks at all (git would not run them either) and gets the in
//! process validator of D3.3 instead, which
//! `joy-core/tests/commit_msg_rule.rs` covers.

use std::path::Path;
use std::process::{Command, Output};

/// Run `joy` in `root` with an isolated home, so nothing of this
/// machine answers for the project under test.
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

fn bash() -> Option<&'static str> {
    ["/bin/bash", "/usr/bin/bash", "bash"]
        .into_iter()
        .find(|c| {
            Command::new(c)
                .arg("-c")
                .arg("exit 0")
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

/// Run joy's commit-msg hook over `message` the way git runs it: from
/// the top of the working tree, with the message file as the argument.
fn commit_msg_hook(shell: &str, root: &Path, message: &str) -> Output {
    let msg_file = root.join(".git/COMMIT_EDITMSG");
    std::fs::write(&msg_file, message).unwrap();
    Command::new(shell)
        .arg(".joy/hooks/commit-msg")
        .arg(&msg_file)
        .current_dir(root)
        .output()
        .expect("the hook runs")
}

#[test]
fn a_repository_with_husky_keeps_its_hooks_and_joys_check_runs_first() {
    let Some(shell) = bash() else {
        eprintln!("no bash on this machine: the hooks would not run here either");
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&home).unwrap();

    // A repository that already has husky: its own hook directory, and
    // `core.hooksPath` pointing at it.
    let repo = git2::Repository::init(&root).unwrap();
    std::fs::create_dir_all(root.join(".husky")).unwrap();
    let husky = root.join(".husky/commit-msg");
    std::fs::write(
        &husky,
        "#!/usr/bin/env bash\necho \"husky saw: $(head -1 \"$1\")\" >> husky.log\nexit 0\n",
    )
    .unwrap();
    make_executable(&husky);
    repo.config()
        .unwrap()
        .set_str("core.hooksPath", ".husky")
        .unwrap();

    let init = joy(
        &root,
        &home,
        &["init", "--name", "Hooked", "--user", "scotty@example.com"],
    );
    assert!(init.status.success(), "{}", text(&init));

    // joy took the path, recorded the one it replaced, and said so once.
    let config = git2::Repository::open(&root).unwrap().config().unwrap();
    assert_eq!(
        config.get_string("core.hooksPath").unwrap(),
        ".joy/hooks",
        "joy owns core.hooksPath"
    );
    let chained = std::fs::read_to_string(root.join(".joy/hooks/chained-path")).unwrap();
    assert_eq!(chained.trim(), ".husky");
    assert!(
        text(&init)
            .contains("joy installed its hooks and kept yours: .husky still runs after joy's."),
        "{}",
        text(&init)
    );

    // A message joy's rule accepts: joy's check passes and husky's hook
    // runs after it.
    let accepted = commit_msg_hook(shell, &root, "chore: wire the hooks [no-item]\n");
    assert!(accepted.status.success(), "{:?}", text(&accepted));
    let log = std::fs::read_to_string(root.join("husky.log")).unwrap();
    assert!(
        log.contains("husky saw: chore: wire the hooks [no-item]"),
        "husky's own hook ran: {log:?}"
    );

    // A message joy's rule refuses: joy's check runs FIRST, so the
    // commit is refused and husky is never reached. If the order were
    // the other way round, husky would have logged a message git is
    // about to throw away.
    let before = std::fs::read_to_string(root.join("husky.log")).unwrap();
    let refused = commit_msg_hook(shell, &root, "wire the hooks\n");
    assert!(!refused.status.success(), "{}", text(&refused));
    let said = String::from_utf8_lossy(&refused.stderr).to_string();
    assert!(said.contains("must reference a Joy item"), "{said}");
    let after = std::fs::read_to_string(root.join("husky.log")).unwrap();
    assert_eq!(before, after, "husky must not run after joy refused");
}

/// A repository that had no `core.hooksPath` at all records nothing and
/// says nothing: joy's hooks then chain to git's own `$GIT_DIR/hooks`,
/// which is exactly what git would have run.
#[test]
fn a_repository_without_a_hook_path_chains_to_gits_own() {
    let Some(shell) = bash() else {
        eprintln!("no bash on this machine: the hooks would not run here either");
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    git2::Repository::init(&root).unwrap();

    let init = joy(
        &root,
        &home,
        &["init", "--name", "Plain", "--user", "scotty@example.com"],
    );
    assert!(init.status.success(), "{}", text(&init));
    assert!(
        !root.join(".joy/hooks/chained-path").exists(),
        "nothing to chain to, nothing recorded"
    );
    assert!(
        !text(&init).contains("kept yours"),
        "nothing was taken over, so nothing is claimed: {}",
        text(&init)
    );

    // git's own hooks directory is where the chain goes.
    let own = root.join(".git/hooks/commit-msg");
    std::fs::create_dir_all(own.parent().unwrap()).unwrap();
    std::fs::write(&own, "#!/usr/bin/env bash\necho ran >> own.log\nexit 0\n").unwrap();
    make_executable(&own);

    let accepted = commit_msg_hook(shell, &root, "chore: nothing to see [no-item]\n");
    assert!(accepted.status.success(), "{}", text(&accepted));
    assert!(
        std::fs::read_to_string(root.join("own.log")).is_ok(),
        "the hook git itself would have run still runs"
    );
}

/// The chained hook's exit code is the hook's, once joy's own check has
/// passed: a repository whose own hook refuses keeps refusing.
#[test]
fn the_chained_hooks_refusal_is_the_hooks_refusal() {
    let Some(shell) = bash() else {
        eprintln!("no bash on this machine: the hooks would not run here either");
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let repo = git2::Repository::init(&root).unwrap();
    std::fs::create_dir_all(root.join(".husky")).unwrap();
    let husky = root.join(".husky/commit-msg");
    std::fs::write(
        &husky,
        "#!/usr/bin/env bash\necho 'husky: no' >&2\nexit 3\n",
    )
    .unwrap();
    make_executable(&husky);
    repo.config()
        .unwrap()
        .set_str("core.hooksPath", ".husky")
        .unwrap();

    let init = joy(
        &root,
        &home,
        &["init", "--name", "Strict", "--user", "scotty@example.com"],
    );
    assert!(init.status.success(), "{}", text(&init));

    let out = commit_msg_hook(shell, &root, "chore: joy is happy [no-item]\n");
    assert_eq!(out.status.code(), Some(3), "{}", text(&out));
    assert!(String::from_utf8_lossy(&out.stderr).contains("husky: no"));
}
