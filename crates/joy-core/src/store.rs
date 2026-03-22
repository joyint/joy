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
}
