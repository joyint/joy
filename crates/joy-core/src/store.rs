// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::JoyError;

pub const JOY_DIR: &str = ".joy";
pub const CONFIG_FILE: &str = "config.yaml";
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
    dir.join(CONFIG_FILE).is_file() && dir.join(PROJECT_FILE).is_file()
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

pub fn read_yaml<T: DeserializeOwned>(path: &Path) -> Result<T, JoyError> {
    let content = std::fs::read_to_string(path).map_err(|e| JoyError::ReadFile {
        path: path.to_path_buf(),
        source: e,
    })?;
    serde_yml::from_str(&content).map_err(JoyError::Yaml)
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
    fn find_project_root_not_found() {
        let dir = tempdir().unwrap();
        assert!(find_project_root(dir.path()).is_none());
    }
}
