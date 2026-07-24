// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! 2026-07 Interaction Levels 2.0 hard switch (JI-0166-D8, JOY-0221-09).
//!
//! The five interaction levels shrink to three and the config keys carry
//! the full term: `interaction:` becomes `interaction-level:`, the member
//! capability floor `max-interaction:` becomes `max-interaction-level:`,
//! and the per-item override `mode:` becomes `interaction-level:`. Values
//! map `pairing`/`interactive`/`collaborative` -> `proposing`,
//! `supervised` -> `confirmed`; `autonomous` is unchanged. After this
//! reconcile the old keys and values are a hard error (the enum refuses
//! them with a `joy update` pointer), so `joy update` must run before any
//! other command touches pre-2.0 data.
//!
//! Files touched under `.joy/`:
//! - `config.yaml` / `config.defaults.yaml`: `interaction:` ->
//!   `interaction-level:` plus value map.
//! - `project.yaml`: same section rename, and every member capability's
//!   `max-interaction:` -> `max-interaction-level:` plus value map.
//! - `items/*.yaml`: `mode:` -> `interaction-level:` plus value map.
//!
//! Chained AFTER `m_2026_07_mode_to_interaction` so a repo still on the
//! ancient `modes:`/`max-mode:` keys migrates through the `interaction`
//! stage in the same `joy update` run. Idempotent: a document already on
//! the new names/values is a no-op; when both the old and new key are
//! present the old one is dropped (the new name wins). Remove this module
//! and its entry in `repo::apply` / `repo::pending` after the deprecation
//! window.

use std::path::{Path, PathBuf};

use serde_yaml_ng::{Mapping, Value};

use super::Reconciled;
use crate::error::JoyError;
use crate::store;

const K_CONFIG_SECTION: &str = ".joy/config.yaml: interaction";
const K_DEFAULTS_SECTION: &str = ".joy/config.defaults.yaml: interaction";
const K_PROJECT_SECTION: &str = ".joy/project.yaml: interaction";
const K_PROJECT_MAX: &str = ".joy/project.yaml: max-interaction";
const K_ITEM_MODE: &str = ".joy/items: mode";
const TO_SECTION: &str = "interaction-level (three levels)";
const TO_MAX: &str = "max-interaction-level (three levels)";
const TO_ITEM: &str = "interaction-level (three levels)";

/// Map a pre-2.0 level value to its three-level successor. Returns `None`
/// when the value is not a known five-level name (already migrated values
/// and garbage are both left untouched).
fn map_level(value: &Value) -> Option<Value> {
    let s = value.as_str()?;
    let mapped = match s {
        "pairing" | "interactive" | "collaborative" => "proposing",
        "supervised" => "confirmed",
        _ => return None,
    };
    Some(Value::String(mapped.into()))
}

/// Map every level value inside a flat interaction mapping (`default` plus
/// per-capability keys). Returns whether anything changed.
fn map_levels_in_section(section: &mut Value) -> bool {
    let Some(map) = section.as_mapping_mut() else {
        return false;
    };
    let mut changed = false;
    for (_key, value) in map.iter_mut() {
        if let Some(mapped) = map_level(value) {
            *value = mapped;
            changed = true;
        }
    }
    changed
}

/// Rename a top-level `interaction:` section to `interaction-level:` and
/// map its values. Returns whether the document changed. If the new key
/// already exists the stray old section is dropped (new name wins), but
/// its values are still mapped in place first so nothing pre-2.0 survives.
fn rename_section(map: &mut Mapping) -> bool {
    let mut changed = false;
    if let Some(mut value) = map.remove("interaction") {
        map_levels_in_section(&mut value);
        if !map.contains_key("interaction-level") {
            map.insert(Value::String("interaction-level".into()), value);
        }
        changed = true;
    }
    if let Some(existing) = map.get_mut("interaction-level") {
        changed |= map_levels_in_section(existing);
    }
    changed
}

/// Rename every member capability's `max-interaction:` floor to
/// `max-interaction-level:` and map its value; also map an already-renamed
/// floor still carrying a five-level value. Returns whether anything changed.
fn rename_member_floors(root: &mut Mapping) -> bool {
    let Some(members) = root.get_mut("members").and_then(Value::as_mapping_mut) else {
        return false;
    };
    let mut changed = false;
    for (_id, member) in members.iter_mut() {
        let member_map = match member.as_mapping_mut() {
            Some(m) => m,
            None => continue,
        };
        // A member-level `interaction-level:` default written by hand with an
        // old value is mapped too.
        if let Some(level) = member_map.get_mut("interaction-level") {
            if let Some(mapped) = map_level(level) {
                *level = mapped;
                changed = true;
            }
        }
        let Some(caps) = member_map
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
            if let Some(mut value) = cfg_map.remove("max-interaction") {
                if let Some(mapped) = map_level(&value) {
                    value = mapped;
                }
                if !cfg_map.contains_key("max-interaction-level") {
                    cfg_map.insert(Value::String("max-interaction-level".into()), value);
                }
                changed = true;
            }
            for key in ["max-interaction-level", "interaction-level"] {
                if let Some(value) = cfg_map.get_mut(key) {
                    if let Some(mapped) = map_level(value) {
                        *value = mapped;
                        changed = true;
                    }
                }
            }
        }
    }
    changed
}

/// Rename an item's `mode:` override to `interaction-level:` and map its
/// value. Returns whether the document changed.
fn rename_item_mode(map: &mut Mapping) -> bool {
    let mut changed = false;
    if let Some(mut value) = map.remove("mode") {
        if let Some(mapped) = map_level(&value) {
            value = mapped;
        }
        if !map.contains_key("interaction-level") {
            map.insert(Value::String("interaction-level".into()), value);
        }
        changed = true;
    }
    if let Some(existing) = map.get_mut("interaction-level") {
        if let Some(mapped) = map_level(existing) {
            *existing = mapped;
            changed = true;
        }
    }
    changed
}

/// Parse a YAML file into its top-level mapping value, or `None` when the
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

fn save(path: &Path, value: &Value) -> Result<(), JoyError> {
    let rendered = serde_yaml_ng::to_string(value)?;
    std::fs::write(path, rendered).map_err(|e| JoyError::WriteFile {
        path: path.to_path_buf(),
        source: e,
    })
}

/// The three config-level files this reconcile rewrites.
struct Target {
    path: PathBuf,
    is_project: bool,
    section_key: &'static str,
}

fn targets(root: &Path) -> Vec<Target> {
    vec![
        Target {
            path: store::local_config_path(root),
            is_project: false,
            section_key: K_CONFIG_SECTION,
        },
        Target {
            path: store::defaults_config_path(root),
            is_project: false,
            section_key: K_DEFAULTS_SECTION,
        },
        Target {
            path: store::joy_dir(root).join(store::PROJECT_FILE),
            is_project: true,
            section_key: K_PROJECT_SECTION,
        },
    ]
}

fn item_paths(root: &Path) -> Vec<PathBuf> {
    let dir = store::joy_dir(root).join("items");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "yaml"))
        .collect();
    out.sort();
    out
}

/// Run the config/project transforms on `value`, collecting the reconciles
/// that applied. Mutates `value` in place.
fn transform(value: &mut Value, target: &Target) -> Vec<Reconciled> {
    let Some(map) = value.as_mapping_mut() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if rename_section(map) {
        out.push(Reconciled {
            key: target.section_key,
            to: TO_SECTION,
        });
    }
    if target.is_project && rename_member_floors(map) {
        out.push(Reconciled {
            key: K_PROJECT_MAX,
            to: TO_MAX,
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
    for path in item_paths(root) {
        if let Some(mut value) = load(&path)? {
            if let Some(map) = value.as_mapping_mut() {
                if rename_item_mode(map) {
                    out.push(Reconciled {
                        key: K_ITEM_MODE,
                        to: TO_ITEM,
                    });
                    break; // one pending entry stands for all item files
                }
            }
        }
    }
    Ok(out)
}

/// Apply the three-level switch to the files at `root`.
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
        save(&target.path, &value)?;
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
    let mut items_changed = false;
    for path in item_paths(root) {
        let Some(mut value) = load(&path)? else {
            continue;
        };
        let Some(map) = value.as_mapping_mut() else {
            continue;
        };
        if !rename_item_mode(map) {
            continue;
        }
        save(&path, &value)?;
        let rel = format!(
            "{}/items/{}",
            store::JOY_DIR,
            path.file_name().and_then(|n| n.to_str()).unwrap_or("")
        );
        crate::git_ops::auto_git_add(root, &[&rel]);
        items_changed = true;
    }
    if items_changed {
        out.push(Reconciled {
            key: K_ITEM_MODE,
            to: TO_ITEM,
        });
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

    fn write_item(root: &Path, name: &str, body: &str) {
        let items = store::joy_dir(root).join("items");
        fs::create_dir_all(&items).unwrap();
        fs::write(items.join(name), body).unwrap();
    }

    fn read(root: &Path, name: &str) -> Value {
        let raw = fs::read_to_string(store::joy_dir(root).join(name)).unwrap();
        serde_yaml_ng::from_str(&raw).unwrap()
    }

    #[test]
    fn renames_and_maps_config_section() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            store::CONFIG_FILE,
            "version: 1\ninteraction:\n  default: pairing\n",
        );
        let done = migrate(dir.path()).unwrap();
        assert_eq!(done.len(), 1);
        let v = read(dir.path(), store::CONFIG_FILE);
        assert!(v.get("interaction").is_none());
        assert_eq!(
            v.get("interaction-level")
                .unwrap()
                .get("default")
                .unwrap()
                .as_str(),
            Some("proposing")
        );
    }

    #[test]
    fn maps_supervised_to_confirmed_in_defaults() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            store::CONFIG_DEFAULTS_FILE,
            "version: 1\ninteraction:\n  default: supervised\n",
        );
        migrate(dir.path()).unwrap();
        let v = read(dir.path(), store::CONFIG_DEFAULTS_FILE);
        assert_eq!(
            v.get("interaction-level")
                .unwrap()
                .get("default")
                .unwrap()
                .as_str(),
            Some("confirmed")
        );
    }

    #[test]
    fn renames_project_section_and_member_floors() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            store::PROJECT_FILE,
            "name: Test\nlanguage: en\ninteraction:\n  implement: collaborative\n  test: supervised\n\
             members:\n  ai:claude@joy:\n    capabilities:\n      implement:\n        \
             max-interaction: interactive\n        max-cost-per-job: 5.0\n",
        );
        let done = migrate(dir.path()).unwrap();
        assert_eq!(done.len(), 2);
        let v = read(dir.path(), store::PROJECT_FILE);
        assert!(v.get("interaction").is_none());
        let section = v.get("interaction-level").unwrap();
        assert_eq!(
            section.get("implement").unwrap().as_str(),
            Some("proposing")
        );
        assert_eq!(section.get("test").unwrap().as_str(), Some("confirmed"));
        let cfg = v
            .get("members")
            .unwrap()
            .get("ai:claude@joy")
            .unwrap()
            .get("capabilities")
            .unwrap()
            .get("implement")
            .unwrap();
        assert!(cfg.get("max-interaction").is_none());
        assert_eq!(
            cfg.get("max-interaction-level").unwrap().as_str(),
            Some("proposing")
        );
        // Unrelated fields survive.
        assert_eq!(cfg.get("max-cost-per-job").unwrap().as_f64(), Some(5.0));
    }

    #[test]
    fn autonomous_survives_unmapped() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            store::CONFIG_FILE,
            "version: 1\ninteraction:\n  default: autonomous\n",
        );
        migrate(dir.path()).unwrap();
        let v = read(dir.path(), store::CONFIG_FILE);
        assert_eq!(
            v.get("interaction-level")
                .unwrap()
                .get("default")
                .unwrap()
                .as_str(),
            Some("autonomous")
        );
    }

    #[test]
    fn maps_values_under_already_renamed_key() {
        // A repo halfway through a manual rename: new key, old values.
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            store::CONFIG_FILE,
            "version: 1\ninteraction-level:\n  default: collaborative\n",
        );
        let done = migrate(dir.path()).unwrap();
        assert_eq!(done.len(), 1);
        let v = read(dir.path(), store::CONFIG_FILE);
        assert_eq!(
            v.get("interaction-level")
                .unwrap()
                .get("default")
                .unwrap()
                .as_str(),
            Some("proposing")
        );
    }

    #[test]
    fn new_key_wins_over_stray_old_section() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            store::CONFIG_FILE,
            "version: 1\ninteraction:\n  default: pairing\ninteraction-level:\n  default: autonomous\n",
        );
        migrate(dir.path()).unwrap();
        let v = read(dir.path(), store::CONFIG_FILE);
        assert!(v.get("interaction").is_none());
        assert_eq!(
            v.get("interaction-level")
                .unwrap()
                .get("default")
                .unwrap()
                .as_str(),
            Some("autonomous")
        );
    }

    #[test]
    fn renames_item_mode_and_maps_value() {
        let dir = tempdir().unwrap();
        write_item(
            dir.path(),
            "TST-0001-aa.yaml",
            "id: TST-0001\ntitle: Test\ntype: task\nstatus: new\npriority: medium\nmode: interactive\n",
        );
        let done = migrate(dir.path()).unwrap();
        assert_eq!(done.len(), 1);
        let raw = fs::read_to_string(
            store::joy_dir(dir.path())
                .join("items")
                .join("TST-0001-aa.yaml"),
        )
        .unwrap();
        let v: Value = serde_yaml_ng::from_str(&raw).unwrap();
        assert!(v.get("mode").is_none());
        assert_eq!(
            v.get("interaction-level").unwrap().as_str(),
            Some("proposing")
        );
    }

    #[test]
    fn item_without_mode_is_untouched() {
        let dir = tempdir().unwrap();
        let body = "id: TST-0002\ntitle: Test\ntype: task\nstatus: new\npriority: medium\n";
        write_item(dir.path(), "TST-0002-bb.yaml", body);
        let done = migrate(dir.path()).unwrap();
        assert!(done.is_empty());
        let raw = fs::read_to_string(
            store::joy_dir(dir.path())
                .join("items")
                .join("TST-0002-bb.yaml"),
        )
        .unwrap();
        assert_eq!(raw, body);
    }

    #[test]
    fn no_op_on_migrated_repo() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            store::PROJECT_FILE,
            "name: Test\nlanguage: en\ninteraction-level:\n  default: proposing\n",
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
            "version: 1\ninteraction:\n  default: pairing\n",
        );
        let p = pending(dir.path()).unwrap();
        assert_eq!(p.len(), 1);
        let v = read(dir.path(), store::CONFIG_FILE);
        assert!(v.get("interaction").is_some());
    }

    #[test]
    fn chains_after_mode_to_interaction() {
        // An ancient repo still on modes:/max-mode: migrates through both
        // stages inside one repo::apply run.
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            store::PROJECT_FILE,
            "name: Test\nlanguage: en\nmodes:\n  implement: interactive\nmembers:\n  \
             ai:claude@joy:\n    capabilities:\n      implement:\n        max-mode: supervised\n",
        );
        super::super::apply(dir.path()).unwrap();
        let v = read(dir.path(), store::PROJECT_FILE);
        assert!(v.get("modes").is_none());
        assert!(v.get("interaction").is_none());
        assert_eq!(
            v.get("interaction-level")
                .unwrap()
                .get("implement")
                .unwrap()
                .as_str(),
            Some("proposing")
        );
        let cfg = v
            .get("members")
            .unwrap()
            .get("ai:claude@joy")
            .unwrap()
            .get("capabilities")
            .unwrap()
            .get("implement")
            .unwrap();
        assert_eq!(
            cfg.get("max-interaction-level").unwrap().as_str(),
            Some("confirmed")
        );
    }
}
