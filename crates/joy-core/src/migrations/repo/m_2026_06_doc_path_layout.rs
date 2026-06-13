// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! 2026-06 doc-path layout reconcile (JOY-01C7-CB).
//!
//! joy's default vision/architecture doc paths changed over time:
//!
//! | Era           | vision                    | architecture                    |
//! | ------------- | ------------------------- | ------------------------------- |
//! | 2026-03 (1)   | docs/dev/Vision.md        | docs/dev/Architecture.md        |
//! | 2026-03 (2)   | docs/dev/vision/README.md | docs/dev/architecture/README.md |
//! | 2026-06 (now) | VISION.md                 | ARCHITECTURE.md                 |
//!
//! joy only persists *non-default* doc paths, so a project that used an
//! earlier default has no `docs.*` entry in project.yaml. Once the
//! default flattened to the repo root, `docs.*_or_default()` would
//! silently repoint such a project at a file that does not exist. This
//! reconcile pins the doc that actually exists on disk into project.yaml
//! so the relocation is lossless.
//!
//! Unlike the pure on-read project_yaml schema migrations, this is a
//! filesystem-aware, one-shot reconcile run at sync time (`joy update` /
//! auto-sync). It edits the raw YAML value so unknown fields round-trip
//! untouched. Idempotent: once `docs.*` is present it is left alone, and
//! a project with no legacy doc on disk is left untouched (the new flat
//! default then applies). Remove this module and its entry in
//! `repo::apply` after the deprecation window.

use std::path::Path;

use serde_yaml_ng::Value;

use super::Reconciled;
use crate::error::JoyError;
use crate::store;

/// Legacy vision doc locations, newest convention first.
const VISION_LEGACY: &[&str] = &[
    "docs/dev/vision/README.md",
    "docs/dev/Vision.md",
    "docs/vision/README.md",
    "docs/vision.md",
];

/// Legacy architecture doc locations, newest convention first.
const ARCHITECTURE_LEGACY: &[&str] = &[
    "docs/dev/architecture/README.md",
    "docs/dev/Architecture.md",
    "docs/architecture/README.md",
    "docs/architecture.md",
];

/// `(yaml key under `docs`, ordered legacy candidate paths)`.
const SPECS: &[(&str, &[&str])] = &[
    ("vision", VISION_LEGACY),
    ("architecture", ARCHITECTURE_LEGACY),
];

fn first_existing(root: &Path, candidates: &[&'static str]) -> Option<&'static str> {
    candidates
        .iter()
        .copied()
        .find(|rel| root.join(rel).is_file())
}

/// Is `docs.<key>` already set in the parsed project.yaml?
fn docs_key_set(value: &Value, key: &str) -> bool {
    value
        .get("docs")
        .and_then(|d| d.get(key))
        .is_some_and(|v| !v.is_null())
}

/// Compute the pins this migration would apply to `value`, without writing.
fn compute(value: &Value, root: &Path) -> Vec<Reconciled> {
    let mut out = Vec::new();
    for (key, candidates) in SPECS {
        if !docs_key_set(value, key) {
            if let Some(to) = first_existing(root, candidates) {
                out.push(Reconciled { key, to });
            }
        }
    }
    out
}

/// Read-only: what this migration would change for the project at `root`.
pub fn pending(root: &Path) -> Result<Vec<Reconciled>, JoyError> {
    match load(root)? {
        Some((value, _)) => Ok(compute(&value, root)),
        None => Ok(Vec::new()),
    }
}

/// Apply the reconcile to project.yaml under `root`. Returns the pins made.
pub fn migrate(root: &Path) -> Result<Vec<Reconciled>, JoyError> {
    let Some((mut value, path)) = load(root)? else {
        return Ok(Vec::new());
    };
    let pins = compute(&value, root);
    if pins.is_empty() {
        return Ok(pins);
    }
    let Some(map) = value.as_mapping_mut() else {
        return Ok(Vec::new());
    };
    let docs = map
        .entry(Value::String("docs".into()))
        .or_insert_with(|| Value::Mapping(Default::default()));
    if let Some(docs_map) = docs.as_mapping_mut() {
        for pin in &pins {
            docs_map.insert(
                Value::String(pin.key.to_string()),
                Value::String(pin.to.to_string()),
            );
        }
    }
    let rendered = serde_yaml_ng::to_string(&value)?;
    std::fs::write(&path, rendered).map_err(|e| JoyError::WriteFile {
        path: path.clone(),
        source: e,
    })?;
    Ok(pins)
}

/// Read project.yaml as a raw YAML value, or `None` when absent.
fn load(root: &Path) -> Result<Option<(Value, std::path::PathBuf)>, JoyError> {
    let path = store::joy_dir(root).join(store::PROJECT_FILE);
    if !path.is_file() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| JoyError::ReadFile {
        path: path.clone(),
        source: e,
    })?;
    let value: Value = serde_yaml_ng::from_str(&raw)?;
    Ok(Some((value, path)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_project(root: &Path, body: &str) {
        let joy = store::joy_dir(root);
        fs::create_dir_all(&joy).unwrap();
        fs::write(joy.join(store::PROJECT_FILE), body).unwrap();
    }

    fn touch(root: &Path, rel: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, "stub").unwrap();
    }

    fn read_docs(root: &Path) -> Value {
        let raw = fs::read_to_string(store::joy_dir(root).join(store::PROJECT_FILE)).unwrap();
        let v: Value = serde_yaml_ng::from_str(&raw).unwrap();
        v.get("docs").cloned().unwrap_or(Value::Null)
    }

    #[test]
    fn pins_era2_nested_readme_when_present() {
        let dir = tempdir().unwrap();
        write_project(dir.path(), "name: Test\nlanguage: en\n");
        touch(dir.path(), "docs/dev/vision/README.md");
        let pins = migrate(dir.path()).unwrap();
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].key, "vision");
        assert_eq!(pins[0].to, "docs/dev/vision/README.md");
        let docs = read_docs(dir.path());
        assert_eq!(
            docs.get("vision").unwrap().as_str(),
            Some("docs/dev/vision/README.md")
        );
    }

    #[test]
    fn pins_era1_capitalised_flat_when_present() {
        let dir = tempdir().unwrap();
        write_project(dir.path(), "name: Test\nlanguage: en\n");
        touch(dir.path(), "docs/dev/Architecture.md");
        let pins = migrate(dir.path()).unwrap();
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].key, "architecture");
        assert_eq!(pins[0].to, "docs/dev/Architecture.md");
    }

    #[test]
    fn prefers_era2_over_era1_when_both_exist() {
        let dir = tempdir().unwrap();
        write_project(dir.path(), "name: Test\nlanguage: en\n");
        touch(dir.path(), "docs/dev/Vision.md");
        touch(dir.path(), "docs/dev/vision/README.md");
        let pins = migrate(dir.path()).unwrap();
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].to, "docs/dev/vision/README.md");
    }

    #[test]
    fn no_op_when_no_legacy_doc_on_disk() {
        let dir = tempdir().unwrap();
        write_project(dir.path(), "name: Test\nlanguage: en\n");
        // Only the new flat default exists -- that is handled by the
        // default mechanism, not this legacy reconcile.
        touch(dir.path(), "VISION.md");
        let pins = migrate(dir.path()).unwrap();
        assert!(pins.is_empty());
        assert!(read_docs(dir.path()).is_null());
    }

    #[test]
    fn idempotent_when_docs_already_set() {
        let dir = tempdir().unwrap();
        write_project(
            dir.path(),
            "name: Test\nlanguage: en\ndocs:\n  vision: custom/V.md\n",
        );
        touch(dir.path(), "docs/dev/vision/README.md");
        let pins = migrate(dir.path()).unwrap();
        assert!(pins.is_empty(), "configured docs.vision must be left alone");
        assert_eq!(
            read_docs(dir.path()).get("vision").unwrap().as_str(),
            Some("custom/V.md")
        );
    }

    #[test]
    fn preserves_unrelated_fields() {
        let dir = tempdir().unwrap();
        write_project(
            dir.path(),
            "name: Test\nlanguage: en\nacronym: TST\ncustom_legacy_field: keep-me\n",
        );
        touch(dir.path(), "docs/dev/vision/README.md");
        migrate(dir.path()).unwrap();
        let raw = fs::read_to_string(store::joy_dir(dir.path()).join(store::PROJECT_FILE)).unwrap();
        assert!(raw.contains("custom_legacy_field: keep-me"));
        assert!(raw.contains("acronym: TST"));
    }
}
