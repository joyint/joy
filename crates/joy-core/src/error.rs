// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum JoyError {
    #[error("project already initialized at {0}")]
    AlreadyInitialized(PathBuf),

    #[error(
        "cannot register the founding member: no git user.email is set and no --user was given.\n\
         Set your identity first, then re-run:\n  git config user.email \"you@example.com\"\n  \
         git config user.name \"Your Name\"\nor: joy init --user you@example.com"
    )]
    NoFounderIdentity,

    #[error("Joy needs Git, but it was not found on your PATH.")]
    GitMissing,

    #[error("no Joy project found (run `joy init` first)")]
    NotInitialized,

    #[error("item not found: {0}")]
    ItemNotFound(String),

    #[error("milestone not found: {0}")]
    MilestoneNotFound(String),

    #[error("circular dependency detected: {0}")]
    CircularDependency(String),

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

    #[error("{path}: {source}")]
    YamlParse {
        path: PathBuf,
        source: serde_yaml_ng::Error,
    },

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml_ng::Error),

    #[error("git error: {0}")]
    Git(String),

    #[error("template error: {0}")]
    Template(String),

    #[error("authentication failed: {0}")]
    AuthFailed(String),

    #[error("crypto error: {0}")]
    Crypto(#[from] joy_crypt::Error),

    #[error("no access to zone '{zone}': ask a member with access to run `joy crypt grant`")]
    ZoneAccessDenied { zone: String },

    #[error("passphrase too short (minimum 3 words)")]
    PassphraseTooShort,

    #[error("guard denied: {0}")]
    GuardDenied(String),

    #[error("{0}")]
    Other(String),
}
