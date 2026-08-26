// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The `release.version-files` list in project.yaml, read and written
//! as RAW YAML.
//!
//! Why raw YAML and not the typed `Project` model: an entry is either a
//! bare path string or a mapping with a `path` field plus arbitrary
//! extra fields. Round-tripping through a typed struct would drop
//! whatever fields the struct does not know, so every mutation here
//! re-parses project.yaml as `serde_yaml_ng::Value`, edits only the
//! `release.version-files` node, and writes the document back --
//! mapping-form entries are preserved verbatim.
//!
//! Lives in joy-core (extracted from joy-cli's project command) so the
//! desktop app and the platform server, which link joy-core as a
//! library, share one implementation of this contract with the CLI.

use anyhow::{bail, Result};

use crate::store;

/// Outcome of [`version_files_add`]: callers word their feedback
/// differently for a fresh entry vs. an idempotent no-op.
pub enum AddOutcome {
    Added,
    AlreadyPresent,
}

/// Extract the path field from a release.version-files entry. Each
/// entry is either a bare string or a mapping with a `path` field
/// (with optional extra fields preserved on round-trip).
pub fn entry_path(entry: &serde_yaml_ng::Value) -> Option<String> {
    use serde_yaml_ng::Value;
    match entry {
        Value::String(s) => Some(s.clone()),
        Value::Mapping(m) => m
            .get(Value::String("path".into()))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

/// Load raw release.version-files entries from project.yaml,
/// preserving mapping-form entries verbatim.
pub fn version_files_raw(root: &std::path::Path) -> Result<Vec<serde_yaml_ng::Value>> {
    let path = store::joy_dir(root).join(store::PROJECT_FILE);
    let raw = std::fs::read_to_string(&path)?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&raw)?;
    let Some(map) = doc.as_mapping() else {
        return Ok(Vec::new());
    };
    let Some(release) = map.get(serde_yaml_ng::Value::String("release".into())) else {
        return Ok(Vec::new());
    };
    let Some(release_map) = release.as_mapping() else {
        return Ok(Vec::new());
    };
    let Some(files) = release_map.get(serde_yaml_ng::Value::String("version-files".into())) else {
        return Ok(Vec::new());
    };
    let Some(seq) = files.as_sequence() else {
        bail!("release.version-files in project.yaml is not a list");
    };
    Ok(seq.clone())
}

/// Read the configured paths as plain strings (mapping-form entries
/// are reduced to their `path` field).
pub fn version_files_get(root: &std::path::Path) -> Result<Vec<String>> {
    Ok(version_files_raw(root)?
        .into_iter()
        .filter_map(|e| entry_path(&e))
        .collect())
}

/// Append `path` to the list unless an entry with that path already
/// exists (idempotent; see [`AddOutcome`]).
pub fn version_files_add(root: &std::path::Path, path: &str) -> Result<AddOutcome> {
    let mut entries = version_files_raw(root)?;
    if entries
        .iter()
        .any(|e| entry_path(e).as_deref() == Some(path))
    {
        return Ok(AddOutcome::AlreadyPresent);
    }
    entries.push(serde_yaml_ng::Value::String(path.to_string()));
    write_version_files_raw(root, entries)?;
    Ok(AddOutcome::Added)
}

/// Remove the entry whose path is `path`. Errors when nothing matches
/// so the caller can surface the typo instead of silently succeeding.
pub fn version_files_rm(root: &std::path::Path, path: &str) -> Result<()> {
    let mut entries = version_files_raw(root)?;
    let before = entries.len();
    entries.retain(|e| entry_path(e).as_deref() != Some(path));
    if entries.len() == before {
        bail!("'{path}' is not configured in release.version-files");
    }
    write_version_files_raw(root, entries)?;
    Ok(())
}

/// Replace the whole list with plain string entries (an empty slice
/// clears it). Mapping-form entries are intentionally NOT synthesized
/// here: whole-list replacement is the "start over" operation.
pub fn version_files_set(root: &std::path::Path, paths: &[String]) -> Result<()> {
    let entries = paths
        .iter()
        .map(|p| serde_yaml_ng::Value::String(p.clone()))
        .collect();
    write_version_files_raw(root, entries)
}

/// Write the supplied entries back to release.version-files,
/// creating the `release:` block if needed and removing
/// `version-files` (or the entire `release:` block if it becomes
/// empty) when entries is empty.
pub fn write_version_files_raw(
    root: &std::path::Path,
    entries: Vec<serde_yaml_ng::Value>,
) -> Result<()> {
    use serde_yaml_ng::Value;
    let path = store::joy_dir(root).join(store::PROJECT_FILE);
    let raw = std::fs::read_to_string(&path)?;
    let mut doc: Value = serde_yaml_ng::from_str(&raw)?;
    let Some(top) = doc.as_mapping_mut() else {
        bail!("project.yaml is not a mapping");
    };
    let release_key = Value::String("release".into());
    let version_key = Value::String("version-files".into());

    if entries.is_empty() {
        // Remove version-files; drop the release block too if it becomes empty.
        if let Some(release) = top.get_mut(&release_key) {
            if let Some(release_map) = release.as_mapping_mut() {
                release_map.remove(&version_key);
                if release_map.is_empty() {
                    top.remove(&release_key);
                }
            }
        }
    } else {
        let release = top
            .entry(release_key)
            .or_insert_with(|| Value::Mapping(Default::default()));
        let Some(release_map) = release.as_mapping_mut() else {
            bail!("project.yaml release: is not a mapping");
        };
        release_map.insert(version_key, Value::Sequence(entries));
    }

    let yaml = serde_yaml_ng::to_string(&doc)?;
    std::fs::write(&path, yaml)?;
    Ok(())
}
