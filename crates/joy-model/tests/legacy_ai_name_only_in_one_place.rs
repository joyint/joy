// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! An AI member's legacy form is spelled out in ONE place (JI-019D-46).
//!
//! `joy_model::migrations::ai_member_name` knows how an AI member was written before it
//! was known by its name, and cuts that down wherever something is read.
//! Nothing else in this repository spells that id out: no code, no test,
//! no help text, no document. This test reads the sources and says where
//! one came back.
//!
//! What may: the module itself, the CLI tests of what a person may still
//! type, and the recorded projects of an earlier release that every
//! later version has to open.

// Nothing of the developer's shell and session reaches this test
// (JOY-02BB-C7).
joy_test_env::isolate!();

use std::path::{Path, PathBuf};

const MAY: &[&str] = &[
    "crates/joy-model/src/migrations/ai_member_name.rs",
    "tests/integration/ai_member_names.bats",
    "tests/fixtures/",
];

const READ: &[&str] = &[
    "rs", "md", "bats", "bash", "sh", "toml", "yaml", "yml", "json",
];

/// The prefix of the legacy form, put together here so that this file does
/// not spell it either.
fn prefix() -> String {
    ["a", "i", ":"].concat()
}

/// Whether `line` writes a legacy form: the prefix as a word of its own,
/// with a name, a placeholder or a closing quote right behind it.
fn spells_one(line: &str, prefix: &str) -> bool {
    line.match_indices(prefix).any(|(at, _)| {
        let before = line[..at].chars().next_back();
        let after = line[at + prefix.len()..].chars().next();
        let starts_a_word = before.is_none_or(|c| !c.is_alphanumeric() && c != '_');
        let names_something =
            after.is_some_and(|c| c.is_ascii_lowercase() || "<{*$\"'`.".contains(c));
        starts_a_word && names_something
    })
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if !name.starts_with('.') && name != "target" && name != "node_modules" {
                sources(&path, out);
            }
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| READ.contains(&e))
        {
            out.push(path);
        }
    }
}

#[test]
fn nothing_but_the_one_module_spells_the_legacy_form() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize().unwrap();
    let prefix = prefix();
    let mut files = Vec::new();
    sources(&root, &mut files);
    assert!(files.len() > 100, "the sources were found");

    let mut found = Vec::new();
    for file in files {
        let shown = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if MAY.iter().any(|may| shown.starts_with(may)) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (number, line) in text.lines().enumerate() {
            if spells_one(line, &prefix) {
                found.push(format!("{shown}:{}: {}", number + 1, line.trim()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "an AI member is its name; only joy_model::migrations::ai_member_name spells the legacy form:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_check_sees_what_it_is_for() {
    let prefix = prefix();
    for line in [
        format!("let id = \"{prefix}claude@joy\";"),
        format!("id.starts_with(\"{prefix}\")"),
        format!("joy assign {prefix}<name>@joy"),
    ] {
        assert!(spells_one(&line, &prefix), "{line}");
    }
    for line in ["openai: yes", "ai: claude", "said: no", "the ai: a tool"] {
        assert!(!spells_one(line, &prefix), "{line}");
    }
}
