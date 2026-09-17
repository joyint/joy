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
//! Membership and order are two of the three things a per crate
//! publish needs; the third is that the versions agree after a bump.
//! `joy release bump` rewrites the version only in the files of
//! `release.version-files` (.joy/project.yaml), so a crate that is in
//! the publish list and in no version file keeps the old version while
//! every dependent is bumped past it, and `cargo publish -p
//! <dependent>` then asks crates.io for a version that was never
//! uploaded. That was true of joy-process, joy-forge-net and
//! joy-telemetry, and none of them may be added to that list from
//! here, because no package of the forge connection NG plan may edit
//! .joy. They inherit instead: `version.workspace = true` against the
//! `[workspace.package]` version of the root manifest, which is in the
//! list, and the two joy-core pins they carry inherit from
//! `[workspace.dependencies]` in the same file.
//! `every_published_crate_has_its_version_bumped` is that third hold.
//!
//! What is NOT proof of any of this: a green
//! `cargo publish --workspace --dry-run`. With `--workspace` cargo
//! verifies each crate against the siblings it just packaged locally,
//! while `just publish-crates` runs `cargo publish -p <crate>` one at a
//! time and each of those resolves its version carrying dependencies
//! against crates.io.
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
        for Dependency {
            name: dependency, ..
        } in internal_dependencies(&text)
        {
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

/// One `joy-*` dependency line of a manifest.
struct Dependency {
    name: String,
    /// Where the version requirement of this edge is written: in this
    /// manifest (`Pinned`), in the workspace manifest (`Inherited`), or
    /// nowhere (`PathOnly`, which cargo refuses to publish at all).
    pin: Pin,
}

#[derive(PartialEq)]
enum Pin {
    Pinned,
    Inherited,
    PathOnly,
}

/// The `joy-*` crates a manifest depends on for its BUILD. The caller
/// keeps the ones that live in this workspace. `[dev-dependencies]` are
/// left out: they carry no version here, so cargo strips them from the
/// published manifest.
fn internal_dependencies(manifest: &str) -> Vec<Dependency> {
    let mut deps = Vec::new();
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
        if !name.starts_with("joy-") {
            continue;
        }
        let pin = if rest.contains("workspace = true") {
            Pin::Inherited
        } else if rest.contains("version") {
            Pin::Pinned
        } else if rest.contains("path") {
            Pin::PathOnly
        } else {
            continue;
        };
        deps.push(Dependency {
            name: name.to_string(),
            pin,
        });
    }
    deps
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
        let manifest = root.join(&member).join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest.display()));
        // The package name out of the manifest, not the directory the
        // manifest lies in: cargo uploads the name and the publish list
        // names it, and the two need not equal the directory.
        let name = package_name(&text)
            .unwrap_or_else(|| panic!("{} names no package", manifest.display()));
        if order.contains(&name) {
            continue;
        }
        if publishes(&text) {
            missing.push(format!(
                "  {name} ({member}) is a workspace member that publish-crates never \
                 uploads and that carries no `publish = false`"
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

/// The `members = [...]` paths of the workspace manifest, in both the
/// multi line form this workspace uses and the inline
/// `members = ["a", "b"]` form, which a reformatting would produce and
/// which an line oriented reader would otherwise swallow whole.
fn workspace_members(manifest: &str) -> Vec<String> {
    let mut members = Vec::new();
    let mut in_members = false;
    for line in manifest.lines().map(str::trim) {
        let mut rest = line;
        if !in_members {
            let Some(after) = line
                .strip_prefix("members")
                .map(str::trim_start)
                .and_then(|a| a.strip_prefix('='))
                .map(str::trim_start)
                .and_then(|a| a.strip_prefix('['))
            else {
                continue;
            };
            in_members = true;
            rest = after;
        }
        let end = rest.find(']');
        let body = match end {
            Some(at) => &rest[..at],
            None => rest,
        };
        for entry in body.split(',') {
            let path = entry.trim().trim_matches('"');
            if !path.is_empty() && !path.starts_with('#') {
                members.push(path.to_string());
            }
        }
        if end.is_some() {
            break;
        }
    }
    members
}

/// The `name` of a manifest's `[package]` section.
fn package_name(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines().map(str::trim) {
        if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            in_package = section == "package";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(value) = line
            .strip_prefix("name")
            .map(str::trim_start)
            .and_then(|v| v.strip_prefix('='))
        {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// Whether a manifest would be uploaded by `cargo publish`, that is,
/// whether it lacks `publish = false`.
fn publishes(manifest: &str) -> bool {
    !manifest
        .lines()
        .map(str::trim)
        .any(|line| line.starts_with("publish") && line.contains("false"))
}

/// The third hold, and the one the two tests above cannot give:
/// `joy release bump` (joy-core/src/version_bump.rs) replaces every
/// quoted occurrence of the current version in the files of
/// `release.version-files` (.joy/project.yaml) and touches no other
/// file. So every version literal a published crate ships has to sit
/// in one of those files: its own `[package] version`, and the
/// requirement of every workspace dependency it carries, because
/// `cargo publish -p <crate>` resolves those against crates.io. A
/// literal that sits anywhere else keeps its old value while its
/// neighbours move past it, which is what JOY-0246-B7 was ("Release
/// bump misses the chat crates: version-files list incomplete") and
/// what JOY-02A4-89 found again for joy-process, joy-forge-net and
/// joy-telemetry.
///
/// A crate has two ways to satisfy this, and the test accepts both:
/// its manifest is in `release.version-files`, or the literal is not
/// in its manifest at all but inherited from the root manifest, which
/// is in that list (`version.workspace = true`,
/// `<dep> = { workspace = true }`). The second way is the one a
/// package that may not edit .joy has.
#[test]
fn every_published_crate_has_its_version_bumped() {
    let root = workspace_root();
    let Ok(justfile) = std::fs::read_to_string(root.join("justfile")) else {
        // A packaged crate carries no justfile, and no workspace either.
        return;
    };
    if !root.join(".joy").join("project.yaml").is_file() {
        // Nor a joy project: this is a source tree check.
        return;
    }
    let order =
        publish_list(&justfile).expect("the publish-crates recipe names a `crates=(...)` list");
    let workspace =
        std::fs::read_to_string(root.join("Cargo.toml")).expect("the workspace manifest");
    let configured = joy_core::version_files::version_files_get(&root)
        .expect("release.version-files in .joy/project.yaml");
    let bumped = |path: &str| configured.iter().any(|entry| covers(entry, path));
    // Every inherited literal lives here, so nothing inherits its way
    // out of the rule.
    let root_is_bumped = bumped("Cargo.toml");

    // The manifest path per package name, so a crate whose directory
    // differs from its name is still looked up by the name the publish
    // list uses.
    let mut manifest_of: HashMap<String, String> = HashMap::new();
    for member in workspace_members(&workspace) {
        let manifest = root.join(&member).join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest.display()));
        if let Some(name) = package_name(&text) {
            manifest_of.insert(name, format!("{member}/Cargo.toml"));
        }
    }

    let mut stale = Vec::new();
    for crate_name in &order {
        let Some(path) = manifest_of.get(crate_name) else {
            continue;
        };
        let text = std::fs::read_to_string(root.join(path))
            .unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
        let own_is_bumped = bumped(path);
        if !(own_is_bumped || (inherits_version(&text) && root_is_bumped)) {
            stale.push(format!(
                "  {path}: its own `version` is a literal in a file the bump does not rewrite"
            ));
        }
        for dependency in internal_dependencies(&text) {
            if !root.join("crates").join(&dependency.name).is_dir() {
                // A sibling repository's crate, published on its own
                // schedule and never bumped by this project.
                continue;
            }
            let covered = match dependency.pin {
                Pin::Inherited => root_is_bumped,
                Pin::Pinned => own_is_bumped,
                // No requirement to go stale. cargo refuses to publish
                // such an edge for a different reason, and says so.
                Pin::PathOnly => true,
            };
            if !covered {
                stale.push(format!(
                    "  {path}: its `{}` requirement is a literal in a file the bump does not rewrite",
                    dependency.name
                ));
            }
        }
    }
    assert!(
        stale.is_empty(),
        "these version literals stay behind when `joy release bump` runs, so the next \
         release resolves a dependency that was never uploaded:\n{}\n\
         Add the manifest to release.version-files in .joy/project.yaml \
         (`joy project set release.version-files --add <path>`), or let the literal \
         inherit from the root manifest, which is in that list \
         (`version.workspace = true`, `<dep> = {{ workspace = true }}`).",
        stale.join("\n")
    );
}

/// Whether a `[package]` takes its version from `[workspace.package]`
/// rather than spelling it out: `version.workspace = true` and
/// `version = { workspace = true }` are the two spellings cargo takes.
fn inherits_version(manifest: &str) -> bool {
    let mut in_package = false;
    for line in manifest.lines().map(str::trim) {
        if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            in_package = section == "package";
            continue;
        }
        if !in_package || !line.starts_with("version") {
            continue;
        }
        if line.contains("workspace") && line.contains("true") {
            return true;
        }
    }
    false
}

/// Whether a `release.version-files` entry names `path`. Entries are
/// plain paths here; a single `*` is honoured because the bump reader
/// expands globs (version_bump.rs `expand_glob`).
fn covers(entry: &str, path: &str) -> bool {
    match entry.split_once('*') {
        None => entry == path,
        Some((prefix, suffix)) => {
            path.len() >= prefix.len() + suffix.len()
                && path.starts_with(prefix)
                && path.ends_with(suffix)
                && !path[prefix.len()..path.len() - suffix.len()].contains('/')
        }
    }
}
