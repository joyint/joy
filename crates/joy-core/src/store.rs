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
pub const LOG_DIR: &str = "log";

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
    let yaml = serde_yml::to_string(value)?;
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
    serde_yml::from_str(&content).ok()
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

    serde_json::from_value(merged).unwrap_or_default()
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

/// Migrate old single-file config to the layered scheme.
///
/// If `.joy/config.yaml` exists but `.joy/config.defaults.yaml` does not,
/// rename the former to the latter.
pub fn migrate_config_if_needed(root: &Path) -> Result<(), JoyError> {
    let local = local_config_path(root);
    let defaults = defaults_config_path(root);

    if local.is_file() && !defaults.is_file() {
        std::fs::rename(&local, &defaults).map_err(|e| JoyError::WriteFile {
            path: defaults,
            source: e,
        })?;
        // Ensure .joy/config.yaml is gitignored after migration
        ensure_gitignore_entry(root, ".joy/config.yaml")?;
    }
    Ok(())
}

/// Add a single entry to .gitignore if not already present.
fn ensure_gitignore_entry(root: &Path, entry: &str) -> Result<(), JoyError> {
    let gitignore_path = root.join(".gitignore");
    let mut content = if gitignore_path.is_file() {
        std::fs::read_to_string(&gitignore_path).map_err(|e| JoyError::ReadFile {
            path: gitignore_path.clone(),
            source: e,
        })?
    } else {
        String::new()
    };

    if content.lines().any(|line| line.trim() == entry) {
        return Ok(());
    }

    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(entry);
    content.push('\n');

    std::fs::write(&gitignore_path, &content).map_err(|e| JoyError::WriteFile {
        path: gitignore_path,
        source: e,
    })
}

pub fn read_yaml<T: DeserializeOwned>(path: &Path) -> Result<T, JoyError> {
    let content = std::fs::read_to_string(path).map_err(|e| JoyError::ReadFile {
        path: path.to_path_buf(),
        source: e,
    })?;
    serde_yml::from_str(&content).map_err(|e| JoyError::YamlParse {
        path: path.to_path_buf(),
        source: e,
    })
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
    fn migrate_renames_config() {
        let dir = tempdir().unwrap();
        let joy = dir.path().join(JOY_DIR);
        std::fs::create_dir_all(&joy).unwrap();
        std::fs::write(joy.join(CONFIG_FILE), "version: 1\n").unwrap();

        migrate_config_if_needed(dir.path()).unwrap();

        assert!(!joy.join(CONFIG_FILE).is_file());
        assert!(joy.join(CONFIG_DEFAULTS_FILE).is_file());
    }

    #[test]
    fn migrate_noop_when_defaults_exists() {
        let dir = tempdir().unwrap();
        let joy = dir.path().join(JOY_DIR);
        std::fs::create_dir_all(&joy).unwrap();
        std::fs::write(joy.join(CONFIG_FILE), "version: 1\n").unwrap();
        std::fs::write(joy.join(CONFIG_DEFAULTS_FILE), "version: 1\n").unwrap();

        migrate_config_if_needed(dir.path()).unwrap();

        // Both files should still exist
        assert!(joy.join(CONFIG_FILE).is_file());
        assert!(joy.join(CONFIG_DEFAULTS_FILE).is_file());
    }
}
