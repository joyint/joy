// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! 2026-07 `mode` -> `interaction` key rename reconcile.
//!
//! The interaction-level config was historically stored under a `modes:`
//! section and a per-capability `max-mode:` floor. Both keys were renamed
//! to `interaction:` / `max-interaction:` across proto, Rust, CLI and the
//! shipped `project.defaults.yaml` template. The embedded default template
//! re-syncs itself, but keys a project already persisted do not: this
//! reconcile rewrites them in place so old repos load losslessly under the
//! new names. It replaces the transient on-read fallback that briefly
//! folded a stray `modes:` back into `interaction:`.
//!
//! Files touched under `.joy/`:
//! - `config.yaml` / `config.defaults.yaml`: top-level `modes:` -> `interaction:`.
//! - `project.yaml`: top-level `modes:` -> `interaction:`, and every
//!   member capability's `max-mode:` -> `max-interaction:`.
//!
//! Unlike the pure on-read project_yaml schema migrations, this is a
//! filesystem-aware, one-shot reconcile run at sync time (`joy update` /
//! auto-sync). It edits the raw YAML value so unknown fields round-trip
//! untouched. Idempotent: a document already on the new names is a no-op,
//! and if both the old and new key are present the old one is dropped
//! (the new name wins). Remove this module and its entry in `repo::apply`
//! / `repo::pending` after the deprecation window.

use std::path::{Path, PathBuf};

use serde_yaml_ng::{Mapping, Value};

use super::Reconciled;
use crate::error::JoyError;
use crate::store;

const K_CONFIG_MODES: &str = ".joy/config.yaml: modes";
const K_DEFAULTS_MODES: &str = ".joy/config.defaults.yaml: modes";
const K_PROJECT_MODES: &str = ".joy/project.yaml: modes";
const K_PROJECT_MAX_MODE: &str = ".joy/project.yaml: max-mode";
const TO_INTERACTION: &str = "interaction";
const TO_MAX_INTERACTION: &str = "max-interaction";

/// Rename a top-level `modes:` key to `interaction:`. Returns whether the
/// document changed. If `interaction:` already exists the stray `modes:`
/// is dropped (new name wins).
fn rename_modes(map: &mut Mapping) -> bool {
    let Some(value) = map.remove("modes") else {
        return false;
    };
    if !map.contains_key("interaction") {
        map.insert(Value::String("interaction".into()), value);
    }
    true
}

/// Rename every member capability's `max-mode:` floor to `max-interaction:`.
/// Returns whether any capability config changed.
fn rename_max_mode(root: &mut Mapping) -> bool {
    let Some(members) = root.get_mut("members").and_then(Value::as_mapping_mut) else {
        return false;
    };
    let mut changed = false;
    for (_id, member) in members.iter_mut() {
        let Some(caps) = member
            .get_mut("capabilities")
            .and_then(Value::as_mapping_mut)
        else {
            // `capabilities: all` is a string, not a map -- nothing to rename.
            continue;
        };
        for (_cap, cfg) in caps.iter_mut() {
            let Some(cfg_map) = cfg.as_mapping_mut() else {
                continue;
            };
            if let Some(value) = cfg_map.remove("max-mode") {
                if !cfg_map.contains_key("max-interaction") {
                    cfg_map.insert(Value::String("max-interaction".into()), value);
                }
                changed = true;
            }
        }
    }
    changed
}

/// One file this reconcile rewrites: its path and the transform to run.
struct Target {
    path: PathBuf,
    /// `true` for project.yaml (also carries member `max-mode` floors).
    is_project: bool,
    modes_key: &'static str,
    max_mode_key: &'static str,
}

fn targets(root: &Path) -> Vec<Target> {
    vec![
        Target {
            path: store::local_config_path(root),
            is_project: false,
            modes_key: K_CONFIG_MODES,
            max_mode_key: "",
        },
        Target {
            path: store::defaults_config_path(root),
            is_project: false,
            modes_key: K_DEFAULTS_MODES,
            max_mode_key: "",
        },
        Target {
            path: store::joy_dir(root).join(store::PROJECT_FILE),
            is_project: true,
            modes_key: K_PROJECT_MODES,
            max_mode_key: K_PROJECT_MAX_MODE,
        },
    ]
}

/// Parse a target file into its top-level YAML mapping, or `None` when the
/// file is absent or not a mapping.
fn load(path: &Path) -> Result<Option<Value>, JoyError> {
    if !path.is_file() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path).map_err(|e| JoyError::ReadFile {
        path: path.to_path_buf(),
        source: e,
    })?;
    let value: Value = serde_yaml_ng::from_str(&raw)?;
    if value.is_mapping() {
        Ok(Some(value))
    } else {
        Ok(None)
    }
}

/// Run the rename transforms on `value`, collecting the reconciles that
/// applied. Mutates `value` in place.
fn transform(value: &mut Value, target: &Target) -> Vec<Reconciled> {
    let Some(map) = value.as_mapping_mut() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if rename_modes(map) {
        out.push(Reconciled {
            key: target.modes_key,
            to: TO_INTERACTION,
        });
    }
    if target.is_project && rename_max_mode(map) {
        out.push(Reconciled {
            key: target.max_mode_key,
            to: TO_MAX_INTERACTION,
        });
    }
    out
}

/// Read-only: what this reconcile would rewrite for the project at `root`.
pub fn pending(root: &Path) -> Result<Vec<Reconciled>, JoyError> {
    let mut out = Vec::new();
    for target in targets(root) {
        if let Some(mut value) = load(&target.path)? {
            out.extend(transform(&mut value, &target));
        }
    }
    Ok(out)
}

/// Apply the key renames to the config/project files at `root`.
pub fn migrate(root: &Path) -> Result<Vec<Reconciled>, JoyError> {
    let mut out = Vec::new();
    for target in targets(root) {
        let Some(mut value) = load(&target.path)? else {
            continue;
        };
        let done = transform(&mut value, &target);
        if done.is_empty() {
            continue;
        }
        let rendered = serde_yaml_ng::to_string(&value)?;
        std::fs::write(&target.path, rendered).map_err(|e| JoyError::WriteFile {
            path: target.path.clone(),
            source: e,
        })?;
        let rel = format!(
            "{}/{}",
            store::JOY_DIR,
            target
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
        );
        crate::git_ops::auto_git_add(root, &[&rel]);
        out.extend(done);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write(root: &Path, name: &str, body: &str) {
        let joy = store::joy_dir(root);
        fs::create_dir_all(&joy).unwrap();
        fs::write(joy.join(name), body).unwrap();
    }

    fn read(root: &Path, name: &str) -> Value {
        let raw = fs::read_to_string(store::joy_dir(root).join(name)).unwrap();
        serde_yaml_ng::from_str(&raw).unwrap()
    }

    #[test]
    fn renames_config_modes_section() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            store::CONFIG_FILE,
            "version: 1\nmodes:\n  default: pairing\n",
        );
        let done = migrate(dir.path()).unwrap();
        assert_eq!(done.len(), 1);
        let v = read(dir.path(), store::CONFIG_FILE);
        assert!(v.get("modes").is_none());
        assert_eq!(
            v.get("interaction")
                .unwrap()
                .get("default")
                .unwrap()
                .as_str(),
            Some("pairing")
        );
    }

    #[test]
    fn renames_defaults_modes_section() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            store::CONFIG_DEFAULTS_FILE,
            "version: 1\nmodes:\n  default: collaborative\n",
        );
        migrate(dir.path()).unwrap();
        let v = read(dir.path(), store::CONFIG_DEFAULTS_FILE);
        assert!(v.get("modes").is_none());
        assert!(v.get("interaction").is_some());
    }

    #[test]
    fn renames_project_modes_and_member_max_mode() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            store::PROJECT_FILE,
            "name: Test\nlanguage: en\nmodes:\n  implement: interactive\nmembers:\n  \
             ai:claude@joy:\n    capabilities:\n      implement:\n        max-mode: supervised\n        \
             max-cost-per-job: 5.0\n",
        );
        let done = migrate(dir.path()).unwrap();
        assert_eq!(done.len(), 2);
        let v = read(dir.path(), store::PROJECT_FILE);
        assert!(v.get("modes").is_none());
        assert!(v.get("interaction").is_some());
        let cfg = v
            .get("members")
            .unwrap()
            .get("ai:claude@joy")
            .unwrap()
            .get("capabilities")
            .unwrap()
            .get("implement")
            .unwrap();
        assert!(cfg.get("max-mode").is_none());
        assert_eq!(
            cfg.get("max-interaction").unwrap().as_str(),
            Some("supervised")
        );
        // Unrelated fields survive.
        assert_eq!(cfg.get("max-cost-per-job").unwrap().as_f64(), Some(5.0));
    }

    #[test]
    fn all_capabilities_string_is_left_untouched() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            store::PROJECT_FILE,
            "name: Test\nlanguage: en\nmembers:\n  horst@joy:\n    capabilities: all\n",
        );
        let done = migrate(dir.path()).unwrap();
        assert!(done.is_empty());
    }

    #[test]
    fn no_op_without_legacy_keys() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            store::PROJECT_FILE,
            "name: Test\nlanguage: en\ninteraction:\n  default: collaborative\n",
        );
        assert!(pending(dir.path()).unwrap().is_empty());
        assert!(migrate(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn pending_reports_without_writing() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            store::CONFIG_FILE,
            "version: 1\nmodes:\n  default: pairing\n",
        );
        let p = pending(dir.path()).unwrap();
        assert_eq!(p.len(), 1);
        // File is untouched.
        let v = read(dir.path(), store::CONFIG_FILE);
        assert!(v.get("modes").is_some());
    }

    #[test]
    fn drops_stray_modes_when_interaction_already_present() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            store::CONFIG_FILE,
            "version: 1\nmodes:\n  default: pairing\ninteraction:\n  default: supervised\n",
        );
        migrate(dir.path()).unwrap();
        let v = read(dir.path(), store::CONFIG_FILE);
        assert!(v.get("modes").is_none());
        // The pre-existing interaction section wins.
        assert_eq!(
            v.get("interaction")
                .unwrap()
                .get("default")
                .unwrap()
                .as_str(),
            Some("supervised")
        );
    }
}
