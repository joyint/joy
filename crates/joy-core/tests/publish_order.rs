// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Lint (JOY-02A3-E4, after JOY-0247-E1): the release publishes the
//! workspace crate by crate, and `cargo publish -p <crate>` resolves
//! every dependency that carries a version against crates.io. A crate
//! whose internal dependency has not been uploaded yet therefore fails
//! mid-release, AFTER the crates before it are irreversibly on the
//! registry.
//!
//! The justfile's `publish-crates` list says the rule out loud ("Order
//! matters: dependents after dependencies") and nothing held the list
//! to it: adding one edge in a Cargo.toml was enough to break the next
//! release, and did. This test is that hold: it reads the list and the
//! manifests and answers the same question cargo will.
//!
//! The exemption this file carried until JOY-02A4-89 is gone with it:
//! joy-cli takes `joy-telemetry = { version = "0.20.0", path =
//! "../joy-telemetry" }` (added 2026-08-28, after the last release)
//! while joy-telemetry rode in neither the publish list nor a
//! `publish = false` key, so `cargo publish -p joy-cli` could not
//! resolve it and the release died after eleven uploads. joy-telemetry
//! is published now, in the list between joy-bi and joy-forge-net, and
//! every such edge fails this test instead of a release.
//!
//! The edge that prompted it was joy-core -> joy-forge-net, added for
//! the shared NO_PROXY matcher while joy-forge-net still rode after
//! joy-core in the list. That edge is gone: the matcher lives in the
//! engine and the connector layer re-exports it, because the layer
//! already depends on the engine for the refresh lock of D2.6a and the
//! second edge would have been a cycle. The rule it exposed stays, and
//! so does its guard.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[test]
fn every_crate_is_published_after_the_crates_it_depends_on() {
    let root = workspace_root();
    let justfile = root.join("justfile");
    let Ok(text) = std::fs::read_to_string(&justfile) else {
        // A packaged crate carries no justfile; there is nothing to
        // check then and nothing to fail either.
        return;
    };
    let order = publish_list(&text).expect("the publish-crates recipe names a `crates=(...)` list");
    let position: HashMap<&str, usize> = order
        .iter()
        .enumerate()
        .map(|(i, name)| (name.as_str(), i))
        .collect();

    let mut violations = Vec::new();
    for (index, crate_name) in order.iter().enumerate() {
        let manifest = root.join("crates").join(crate_name).join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest.display()));
        for dependency in internal_dependencies(&text) {
            // Only this workspace's own crates: joy-crypt and joy-token
            // come from a sibling repository and are published on their
            // own schedule, so this list says nothing about them.
            if !root.join("crates").join(&dependency).is_dir() {
                continue;
            }
            match position.get(dependency.as_str()) {
                Some(&at) if at < index => {}
                Some(&at) => violations.push(format!(
                    "  {crate_name} (position {index}) depends on {dependency}, \
                     which publish-crates uploads later (position {at})"
                )),
                None => violations.push(format!(
                    "  {crate_name} depends on the workspace crate {dependency}, \
                     which publish-crates never uploads"
                )),
            }
        }
    }
    assert!(
        violations.is_empty(),
        "publish-crates would fail halfway through the next release:\n{}\n\
         Move the dependency ahead of its dependent in the justfile's `crates=(...)` list.",
        violations.join("\n")
    );
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name>/ sits two levels below the workspace root")
        .to_path_buf()
}

/// The crate names of the `crates=(a b c)` line in `publish-crates`.
fn publish_list(justfile: &str) -> Option<Vec<String>> {
    let line = justfile
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("crates=("))?;
    let inner = line.trim_start_matches("crates=(").trim_end_matches(')');
    Some(inner.split_whitespace().map(str::to_string).collect())
}

/// The `joy-*` crates a manifest depends on for its BUILD. The caller
/// keeps the ones that live in this workspace. `[dev-dependencies]` are
/// left out: they carry no version here, so cargo strips them from the
/// published manifest.
fn internal_dependencies(manifest: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_dependencies = false;
    for line in manifest.lines().map(str::trim) {
        if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            in_dependencies = section.ends_with("dependencies") && !section.contains("dev-");
            continue;
        }
        if !in_dependencies || line.starts_with('#') {
            continue;
        }
        let Some((name, rest)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.starts_with("joy-") && (rest.contains("workspace = true") || rest.contains("path"))
        {
            names.push(name.to_string());
        }
    }
    names
}

/// The other half of the justfile's rule: "Every workspace member is
/// either in this list or carries publish = false". The order test
/// above only sees crates a published crate depends on, so a new
/// workspace member that nothing depends on yet would sit outside both
/// the list and the rule until someone added the first edge, which is
/// the shape of JOY-0247-E1 and of JOY-02A4-89. This test reads the
/// workspace members and answers the question directly.
#[test]
fn every_workspace_member_is_published_or_marked_unpublishable() {
    let root = workspace_root();
    let Ok(justfile) = std::fs::read_to_string(root.join("justfile")) else {
        // A packaged crate carries no justfile, and no workspace either.
        return;
    };
    let order =
        publish_list(&justfile).expect("the publish-crates recipe names a `crates=(...)` list");
    let workspace =
        std::fs::read_to_string(root.join("Cargo.toml")).expect("the workspace manifest");

    let mut missing = Vec::new();
    for member in workspace_members(&workspace) {
        let name = member
            .rsplit('/')
            .next()
            .expect("a member path ends in the crate directory")
            .to_string();
        if order.contains(&name) {
            continue;
        }
        let manifest = root.join(&member).join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest.display()));
        if publishes(&text) {
            missing.push(format!(
                "  {name} is a workspace member that publish-crates never uploads \
                 and that carries no `publish = false`"
            ));
        }
    }
    assert!(
        missing.is_empty(),
        "the publishable crate set and the publish-crates list disagree:\n{}\n\
         Add the crate to the justfile's `crates=(...)` list in dependency order, \
         or give its manifest `publish = false`.",
        missing.join("\n")
    );
}

/// The `members = [...]` paths of the workspace manifest.
fn workspace_members(manifest: &str) -> Vec<String> {
    let mut members = Vec::new();
    let mut in_members = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with("members") && line.contains('[') {
            in_members = true;
            continue;
        }
        if !in_members {
            continue;
        }
        if line.starts_with(']') {
            break;
        }
        let path = line.trim_end_matches(',').trim_matches('"');
        if !path.is_empty() {
            members.push(path.to_string());
        }
    }
    members
}

/// Whether a manifest would be uploaded by `cargo publish`, that is,
/// whether it lacks `publish = false`.
fn publishes(manifest: &str) -> bool {
    !manifest
        .lines()
        .map(str::trim)
        .any(|line| line.starts_with("publish") && line.contains("false"))
}
