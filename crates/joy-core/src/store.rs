// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::JoyError;

pub const JOY_DIR: &str = ".joy";
pub const CONFIG_FILE: &str = "config.yaml";
pub const CONFIG_DEFAULTS_FILE: &str = "config.defaults.yaml";
pub const PROJECT_FILE: &str = "project.yaml";
pub const PROJECT_DEFAULTS_FILE: &str = "project.defaults.yaml";
pub const CREDENTIALS_FILE: &str = "credentials.yaml";
pub const ITEMS_DIR: &str = "items";
pub const MILESTONES_DIR: &str = "milestones";
pub const AI_DIR: &str = "ai";
pub const AI_AGENTS_DIR: &str = "ai/agents";
pub const AI_JOBS_DIR: &str = "ai/jobs";
pub const LOG_DIR: &str = "logs";
pub const RELEASES_DIR: &str = "releases";

pub fn joy_dir(root: &Path) -> PathBuf {
    root.join(JOY_DIR)
}

pub fn is_initialized(root: &Path) -> bool {
    let dir = joy_dir(root);
    let has_config = dir.join(CONFIG_FILE).is_file() || dir.join(CONFIG_DEFAULTS_FILE).is_file();
    has_config && dir.join(PROJECT_FILE).is_file()
}

/// Walk up from `start` looking for a `.joy/` directory.
pub fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if is_initialized(&current) {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

pub fn write_yaml<T: Serialize>(path: &Path, value: &T) -> Result<(), JoyError> {
    let yaml = serde_yaml_ng::to_string(value)?;
    std::fs::write(path, yaml).map_err(|e| JoyError::WriteFile {
        path: path.to_path_buf(),
        source: e,
    })
}

/// Write YAML while preserving top-level fields not in the struct.
/// Reads the existing file, takes all modeled fields from the struct,
/// and preserves any top-level keys that the struct doesn't know about.
pub fn write_yaml_preserve<T: Serialize>(path: &Path, value: &T) -> Result<(), JoyError> {
    use serde_yaml_ng::Value;

    let new_value: Value = serde_yaml_ng::to_value(value)?;

    let merged = if path.is_file() {
        let existing_str = std::fs::read_to_string(path).map_err(|e| JoyError::ReadFile {
            path: path.to_path_buf(),
            source: e,
        })?;
        if let Ok(existing) = serde_yaml_ng::from_str::<Value>(&existing_str) {
            if let (Value::Mapping(existing_map), Value::Mapping(new_map)) = (existing, &new_value)
            {
                // Start with the struct's values (authoritative for modeled fields)
                let mut result = new_map.clone();
                // Add back any top-level keys from the original that the struct doesn't have
                for (key, val) in existing_map {
                    if !result.contains_key(&key) {
                        result.insert(key, val);
                    }
                }
                Value::Mapping(result)
            } else {
                new_value
            }
        } else {
            new_value
        }
    } else {
        new_value
    };

    let yaml = serde_yaml_ng::to_string(&merged)?;
    std::fs::write(path, yaml).map_err(|e| JoyError::WriteFile {
        path: path.to_path_buf(),
        source: e,
    })
}

/// Returns the path to the personal global config: ~/.config/joy/config.yaml
pub fn global_config_path() -> PathBuf {
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs_path_home()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config")
        });
    config_dir.join("joy").join("config.yaml")
}

/// Returns the path to the personal per-project config: .joy/config.yaml
pub fn local_config_path(root: &Path) -> PathBuf {
    joy_dir(root).join(CONFIG_FILE)
}

/// Returns the path to the committed project defaults: .joy/config.defaults.yaml
pub fn defaults_config_path(root: &Path) -> PathBuf {
    joy_dir(root).join(CONFIG_DEFAULTS_FILE)
}

pub fn project_defaults_path(root: &Path) -> PathBuf {
    joy_dir(root).join(PROJECT_DEFAULTS_FILE)
}

fn dirs_path_home() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

/// Recursively merge `overlay` into `base`. Object keys are merged; all other
/// types are replaced.
/// Recursively merge `overlay` into `base` (public for use by config validation).
pub fn deep_merge_value(base: &mut serde_json::Value, overlay: &serde_json::Value) {
    deep_merge(base, overlay);
}

fn deep_merge(base: &mut serde_json::Value, overlay: &serde_json::Value) {
    if let (Some(base_map), Some(overlay_map)) = (base.as_object_mut(), overlay.as_object()) {
        for (key, value) in overlay_map {
            if let Some(existing) = base_map.get_mut(key) {
                deep_merge(existing, value);
            } else {
                base_map.insert(key.clone(), value.clone());
            }
        }
    } else {
        *base = overlay.clone();
    }
}

/// Read a YAML file as a serde_json::Value, returning None if the file does not exist.
fn read_yaml_value(path: &Path) -> Option<serde_json::Value> {
    let content = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_yaml_ng::from_str(&content).ok()?;
    // Empty YAML files deserialize as null -- treat them as absent
    if value.is_null() {
        return None;
    }
    Some(value)
}

/// Load project config by merging four layers (code defaults < project defaults
/// < global personal < local personal).
pub fn load_config() -> crate::model::Config {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(_) => return crate::model::Config::default(),
    };
    let root = match find_project_root(&cwd) {
        Some(r) => r,
        None => return crate::model::Config::default(),
    };

    // Layer 4: code defaults
    let mut merged: serde_json::Value =
        serde_json::to_value(crate::model::Config::default()).unwrap_or_default();

    // Layer 3: project defaults (.joy/config.defaults.yaml)
    if let Some(defaults) = read_yaml_value(&defaults_config_path(&root)) {
        deep_merge(&mut merged, &defaults);
    }

    // Layer 2: global personal (~/.config/joy/config.yaml)
    if let Some(global) = read_yaml_value(&global_config_path()) {
        deep_merge(&mut merged, &global);
    }

    // Layer 1: local personal (.joy/config.yaml)
    if let Some(local) = read_yaml_value(&local_config_path(&root)) {
        deep_merge(&mut merged, &local);
    }

    match serde_json::from_value(merged) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Warning: config has invalid values, using defaults: {e}");
            crate::model::Config::default()
        }
    }
}

/// Load only the user-set config values (personal local + global, no defaults).
/// Returns an empty object if no personal config exists.
pub fn load_personal_config_value() -> serde_json::Value {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(_) => return serde_json::json!({}),
    };
    let root = match find_project_root(&cwd) {
        Some(r) => r,
        None => return serde_json::json!({}),
    };

    let mut merged = serde_json::json!({});

    if let Some(global) = read_yaml_value(&global_config_path()) {
        deep_merge(&mut merged, &global);
    }
    if let Some(local) = read_yaml_value(&local_config_path(&root)) {
        deep_merge(&mut merged, &local);
    }

    merged
}

/// Load the merged config as a serde_json::Value (preserves arbitrary keys).
pub fn load_config_value() -> serde_json::Value {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(_) => return serde_json::to_value(crate::model::Config::default()).unwrap_or_default(),
    };
    let root = match find_project_root(&cwd) {
        Some(r) => r,
        None => return serde_json::to_value(crate::model::Config::default()).unwrap_or_default(),
    };

    let mut merged: serde_json::Value =
        serde_json::to_value(crate::model::Config::default()).unwrap_or_default();

    if let Some(defaults) = read_yaml_value(&defaults_config_path(&root)) {
        deep_merge(&mut merged, &defaults);
    }
    if let Some(global) = read_yaml_value(&global_config_path()) {
        deep_merge(&mut merged, &global);
    }
    if let Some(local) = read_yaml_value(&local_config_path(&root)) {
        deep_merge(&mut merged, &local);
    }

    merged
}

pub fn read_yaml<T: DeserializeOwned>(path: &Path) -> Result<T, JoyError> {
    let content = std::fs::read_to_string(path).map_err(|e| JoyError::ReadFile {
        path: path.to_path_buf(),
        source: e,
    })?;
    serde_yaml_ng::from_str(&content).map_err(|e| JoyError::YamlParse {
        path: path.to_path_buf(),
        source: e,
    })
}

/// Load the full project metadata from project.yaml.
pub fn load_project(root: &Path) -> Result<crate::model::project::Project, crate::error::JoyError> {
    let project_path = joy_dir(root).join(PROJECT_FILE);
    read_yaml(&project_path)
}

/// Load mode defaults by merging project.defaults.yaml with project.yaml modes section.
pub fn load_mode_defaults(root: &Path) -> crate::model::project::ModeDefaults {
    let defaults_path = project_defaults_path(root);
    let mut base = read_yaml_value(&defaults_path)
        .and_then(|v| v.get("modes").cloned())
        .unwrap_or(serde_json::json!({}));

    // Overlay from project.yaml modes section
    let project_path = joy_dir(root).join(PROJECT_FILE);
    if let Some(overlay) = read_yaml_value(&project_path).and_then(|v| v.get("modes").cloned()) {
        deep_merge(&mut base, &overlay);
    }

    serde_json::from_value(base).unwrap_or_default()
}

/// Load the raw mode defaults from project.defaults.yaml (before project.yaml merge).
/// Used for source tracking in resolve_mode().
pub fn load_raw_mode_defaults(root: &Path) -> crate::model::project::ModeDefaults {
    let path = project_defaults_path(root);
    read_yaml_value(&path)
        .and_then(|v| v.get("modes").cloned())
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// Load AI defaults (capabilities granted to AI members) from project.defaults.yaml,
/// with project.yaml ai-defaults overlay.
pub fn load_ai_defaults(root: &Path) -> crate::model::project::AiDefaults {
    let defaults_path = project_defaults_path(root);
    let mut base = read_yaml_value(&defaults_path)
        .and_then(|v| v.get("ai-defaults").cloned())
        .unwrap_or(serde_json::json!({}));

    let project_path = joy_dir(root).join(PROJECT_FILE);
    if let Some(overlay) =
        read_yaml_value(&project_path).and_then(|v| v.get("ai-defaults").cloned())
    {
        deep_merge(&mut base, &overlay);
    }

    serde_json::from_value(base).unwrap_or_default()
}

/// Load the project acronym from project.yaml.
pub fn load_acronym(root: &Path) -> Result<String, crate::error::JoyError> {
    let project_path = joy_dir(root).join(PROJECT_FILE);
    let project: crate::model::project::Project = read_yaml(&project_path)?;
    project.acronym.ok_or_else(|| {
        crate::error::JoyError::Other(
            "project acronym not set -- run: joy project --acronym <ACRONYM>".to_string(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Config;
    use tempfile::tempdir;

    #[test]
    fn write_and_read_yaml_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.yaml");
        let config = Config::default();
        write_yaml(&path, &config).unwrap();
        let parsed: Config = read_yaml(&path).unwrap();
        assert_eq!(config, parsed);
    }

    #[test]
    fn is_initialized_empty_dir() {
        let dir = tempdir().unwrap();
        assert!(!is_initialized(dir.path()));
    }

    #[test]
    fn is_initialized_with_defaults_file() {
        let dir = tempdir().unwrap();
        let joy = dir.path().join(JOY_DIR);
        std::fs::create_dir_all(&joy).unwrap();
        write_yaml(&joy.join(CONFIG_DEFAULTS_FILE), &Config::default()).unwrap();
        write_yaml(
            &joy.join(PROJECT_FILE),
            &crate::model::project::Project::new("test".into(), None),
        )
        .unwrap();
        assert!(is_initialized(dir.path()));
    }

    #[test]
    fn find_project_root_not_found() {
        let dir = tempdir().unwrap();
        assert!(find_project_root(dir.path()).is_none());
    }

    #[test]
    fn deep_merge_objects() {
        let mut base = serde_json::json!({"a": 1, "b": {"c": 2, "d": 3}});
        let overlay = serde_json::json!({"b": {"c": 99, "e": 4}, "f": 5});
        deep_merge(&mut base, &overlay);
        assert_eq!(
            base,
            serde_json::json!({"a": 1, "b": {"c": 99, "d": 3, "e": 4}, "f": 5})
        );
    }

    #[test]
    fn deep_merge_replaces_non_objects() {
        let mut base = serde_json::json!({"a": [1, 2]});
        let overlay = serde_json::json!({"a": [3]});
        deep_merge(&mut base, &overlay);
        assert_eq!(base, serde_json::json!({"a": [3]}));
    }

    #[test]
    fn read_yaml_value_returns_none_for_empty_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("empty.yaml");
        std::fs::write(&path, "").unwrap();
        assert!(read_yaml_value(&path).is_none());
    }

    #[test]
    fn read_yaml_value_returns_none_for_whitespace_only() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("blank.yaml");
        std::fs::write(&path, "  \n\n").unwrap();
        assert!(read_yaml_value(&path).is_none());
    }

    // -----------------------------------------------------------------------
    // Mode defaults loading integration tests
    // -----------------------------------------------------------------------

    use crate::model::config::InteractionLevel;
    use crate::model::item::Capability;

    fn setup_project_dir(dir: &std::path::Path) {
        let joy = dir.join(JOY_DIR);
        std::fs::create_dir_all(&joy).unwrap();
        let project = crate::model::project::Project::new("test".into(), Some("TST".into()));
        write_yaml(&joy.join(PROJECT_FILE), &project).unwrap();
    }

    #[test]
    fn load_mode_defaults_from_file() {
        let dir = tempdir().unwrap();
        setup_project_dir(dir.path());
        let defaults_content = r#"
modes:
  default: interactive
  implement: collaborative
  review: pairing
"#;
        std::fs::write(
            dir.path().join(JOY_DIR).join(PROJECT_DEFAULTS_FILE),
            defaults_content,
        )
        .unwrap();

        let defaults = load_mode_defaults(dir.path());
        assert_eq!(defaults.default, InteractionLevel::Interactive);
        assert_eq!(
            defaults.capabilities[&Capability::Implement],
            InteractionLevel::Collaborative
        );
        assert_eq!(
            defaults.capabilities[&Capability::Review],
            InteractionLevel::Pairing
        );
    }

    #[test]
    fn load_mode_defaults_missing_file_returns_default() {
        let dir = tempdir().unwrap();
        setup_project_dir(dir.path());
        let defaults = load_mode_defaults(dir.path());
        assert_eq!(defaults.default, InteractionLevel::Collaborative);
        assert!(defaults.capabilities.is_empty());
    }

    #[test]
    fn load_mode_defaults_project_yaml_overrides() {
        let dir = tempdir().unwrap();
        setup_project_dir(dir.path());

        let defaults_content = r#"
modes:
  default: collaborative
  implement: collaborative
"#;
        std::fs::write(
            dir.path().join(JOY_DIR).join(PROJECT_DEFAULTS_FILE),
            defaults_content,
        )
        .unwrap();

        // project.yaml overrides implement to interactive
        let project_content = r#"
name: test
acronym: TST
language: en
created: "2026-01-01T00:00:00+00:00"
members: {}
modes:
  implement: interactive
"#;
        std::fs::write(dir.path().join(JOY_DIR).join(PROJECT_FILE), project_content).unwrap();

        let defaults = load_mode_defaults(dir.path());
        assert_eq!(
            defaults.capabilities[&Capability::Implement],
            InteractionLevel::Interactive
        );
    }

    #[test]
    fn load_raw_mode_defaults_ignores_project_overrides() {
        let dir = tempdir().unwrap();
        setup_project_dir(dir.path());

        let defaults_content = r#"
modes:
  implement: collaborative
"#;
        std::fs::write(
            dir.path().join(JOY_DIR).join(PROJECT_DEFAULTS_FILE),
            defaults_content,
        )
        .unwrap();

        let project_content = r#"
name: test
acronym: TST
language: en
created: "2026-01-01T00:00:00+00:00"
members: {}
modes:
  implement: interactive
"#;
        std::fs::write(dir.path().join(JOY_DIR).join(PROJECT_FILE), project_content).unwrap();

        let raw = load_raw_mode_defaults(dir.path());
        assert_eq!(
            raw.capabilities[&Capability::Implement],
            InteractionLevel::Collaborative
        );
    }

    #[test]
    fn load_ai_defaults_from_file() {
        let dir = tempdir().unwrap();
        setup_project_dir(dir.path());

        let defaults_content = r#"
ai-defaults:
  capabilities:
    - implement
    - review
    - plan
"#;
        std::fs::write(
            dir.path().join(JOY_DIR).join(PROJECT_DEFAULTS_FILE),
            defaults_content,
        )
        .unwrap();

        let defaults = load_ai_defaults(dir.path());
        assert_eq!(defaults.capabilities.len(), 3);
    }

    #[test]
    fn load_ai_defaults_project_override_replaces_capabilities() {
        let dir = tempdir().unwrap();
        setup_project_dir(dir.path());

        let defaults_content = r#"
ai-defaults:
  capabilities:
    - implement
    - review
    - plan
"#;
        std::fs::write(
            dir.path().join(JOY_DIR).join(PROJECT_DEFAULTS_FILE),
            defaults_content,
        )
        .unwrap();

        let project_content = r#"
name: test
acronym: TST
language: en
created: "2026-01-01T00:00:00+00:00"
members: {}
ai-defaults:
  capabilities:
    - implement
"#;
        std::fs::write(dir.path().join(JOY_DIR).join(PROJECT_FILE), project_content).unwrap();

        let defaults = load_ai_defaults(dir.path());
        assert_eq!(defaults.capabilities, vec![Capability::Implement]);
    }
}
