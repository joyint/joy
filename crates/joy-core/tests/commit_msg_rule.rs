// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The item reference rule, once, for both readers (design D3.3).
//!
//! libgit2 runs no hooks, so every commit joy writes for itself passes
//! `joy_core::commit_msg::validate` instead of `.joy/hooks/commit-msg`.
//! The hook stays for the person's own `git commit` on a machine that
//! has git. Two enforcers of one rule drift, so this test holds them
//! against the same corpus: for every message below, the hook's exit
//! code and the validator's answer agree.
//!
//! The hook is bash and needs a shell. Where there is none (Windows
//! without Git for Windows' bundled sh) the test says so and skips the
//! bash half; the validator half runs everywhere, which is the point of
//! D3.3 in the first place.

use std::path::Path;
use std::process::Command;

const ACRONYM: &str = "JOY";

/// The corpus: every branch of the hook, and the two that used to be
/// read differently by the two implementations.
const CASES: &[&str] = &[
    // an id in the subject, both spellings of ADR-027
    "feat: [JOY-0005-EE] do it",
    "feat: JOY-0005 do it",
    // lower case hex is hex
    "joy: add a task\n\nrefs JOY-00ab",
    // the bypass, anywhere in the message
    "chore: bump the app [no-item]",
    "chore: bump the app\n\nnothing here is an item [no-item]\n",
    // too few digits
    "feat: JOY-12 do it",
    // another project's acronym
    "feat: [ANO-0005-EE] do it",
    // nothing at all
    "joy: auth passphrase",
    // an id-shaped word that is not this project's
    "fix: JOYFUL-0001 is not an id",
    // the four digits must be hex
    "feat: JOY-12g4 do it",
];

fn shell() -> Option<&'static str> {
    for candidate in ["/bin/bash", "/usr/bin/bash", "bash"] {
        if Command::new(candidate)
            .arg("-c")
            .arg("exit 0")
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return Some(candidate);
        }
    }
    None
}

/// Run the shipped hook over `message` in a directory that looks like a
/// Joy project, and answer whether it accepted.
fn hook_accepts(shell: &str, dir: &Path, message: &str) -> bool {
    let msg_file = dir.join("COMMIT_EDITMSG");
    std::fs::write(&msg_file, message).unwrap();
    let hook = dir.join("commit-msg");
    Command::new(shell)
        .arg(&hook)
        .arg(&msg_file)
        .current_dir(dir)
        .output()
        .expect("the hook runs")
        .status
        .success()
}

#[test]
fn the_hook_and_the_validator_answer_the_same_corpus() {
    let Some(shell) = shell() else {
        eprintln!("no bash on this machine: only the in-process rule is checked here");
        for message in CASES {
            // The validator still has to be total: no case may panic.
            let _ = joy_core::commit_msg::validate(message, ACRONYM);
        }
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("commit-msg"),
        include_str!("../data/hooks/commit-msg"),
    )
    .unwrap();
    // The chain tail of D3.5 rides with the hook: without it the hook
    // stops at joy's own verdict, which is the same answer here (there
    // is nothing to chain to in this directory) but a different code
    // path, and the test has to exercise the one that ships.
    let chainer = root.join("joy-chain");
    std::fs::write(&chainer, include_str!("../data/hooks/joy-chain")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&chainer, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::create_dir_all(root.join(".joy")).unwrap();
    std::fs::write(
        root.join(".joy/project.yaml"),
        format!("name: Corpus\nacronym: {ACRONYM}\n"),
    )
    .unwrap();

    for message in CASES {
        let hook = hook_accepts(shell, root, message);
        let validator = joy_core::commit_msg::validate(message, ACRONYM).is_ok();
        assert_eq!(
            hook, validator,
            "the hook and the in-process rule disagree about {message:?}: \
             hook accepted = {hook}, validator accepted = {validator}"
        );
    }
}

/// A project without an acronym has no rule, and both readers exit 0 on
/// every message. The hook reads the acronym out of `.joy/project.yaml`
/// and the validator is given it; an empty one is the same "no rule" on
/// both sides.
#[test]
fn a_project_without_an_acronym_has_no_rule() {
    for message in CASES {
        assert!(
            joy_core::commit_msg::validate(message, "").is_ok(),
            "{message:?} must pass where there is no acronym"
        );
        assert!(joy_core::commit_msg::validate(message, "   ").is_ok());
    }
}
