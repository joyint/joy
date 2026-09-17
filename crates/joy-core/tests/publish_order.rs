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
//! release, and did (joy-core gained joy-forge-net while joy-forge-net
//! still rode after joy-core). This test is that hold: it reads the
//! list and the manifests and answers the same question cargo will.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Workspace crates a published crate depends on that `publish-crates`
/// does NOT upload. There is one, and it is a release breaker that
/// predates this test: joy-cli takes `joy-telemetry = { version =
/// "0.20.0", path = "../joy-telemetry" }` (added 2026-08-28, after the
/// last release), joy-telemetry is in neither the publish list nor
/// marked `publish = false`, and no such crate exists on crates.io, so
/// `cargo publish -p joy-cli` cannot resolve it. Fixing it means
/// uploading a new public crate name, which is the operator's call, so
/// it is named here instead of hidden: every OTHER such edge fails this
/// test at once.
const NOT_UPLOADED_YET: &[&str] = &["joy-telemetry"];

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
                None if NOT_UPLOADED_YET.contains(&dependency.as_str()) => {}
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
