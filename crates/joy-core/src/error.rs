// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum JoyError {
    #[error("project already initialized at {0}")]
    AlreadyInitialized(PathBuf),

    #[error("failed to create directory {path}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to write {path}")]
    WriteFile {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to read {path}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yml::Error),

    #[error("git error: {0}")]
    Git(String),
}
