// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Command context: identity, guard, project root.
//!
//! Loaded once per command, provides all the infrastructure that
//! write operations need: who is acting, what are they allowed to do,
//! and where is the project.

use std::path::PathBuf;

use crate::error::JoyError;
use crate::guard::{Action, Guard};
use crate::identity::{self, Identity};
use crate::store;

/// Shared context for CLI commands.
pub struct Context {
    pub root: PathBuf,
    pub identity: Identity,
    guard: Guard,
}

impl Context {
    /// Load context from the current directory.
    /// Finds the project root, resolves identity from session or git email,
    /// and loads Guard with gate config.
    pub fn load() -> Result<Self, JoyError> {
        let cwd =
            std::env::current_dir().map_err(|e| JoyError::Other(format!("current dir: {e}")))?;
        let root = store::find_project_root(&cwd).ok_or(JoyError::NotInitialized)?;
        Self::load_at(&root)
    }

    /// Load the context of the project at `root`.
    pub fn load_at(root: &std::path::Path) -> Result<Self, JoyError> {
        let root = root.to_path_buf();
        let identity = identity::resolve_identity(&root).unwrap_or(Identity {
            member: "unknown".into(),
            delegated_by: None,
            authenticated: false,
            grant: None,
        });
        let guard = Guard::load(&root)?;
        Ok(Self {
            root,
            identity,
            guard,
        })
    }

    /// The guard this context enforces with, for a host that wants to
    /// ask a question of it (whether proof is needed) before enforcing.
    pub fn guard(&self) -> &Guard {
        &self.guard
    }

    /// Check and enforce a guard action. Logs events on deny/warn.
    pub fn enforce(&self, action: &Action, target: &str) -> Result<(), JoyError> {
        self.guard
            .check(action, &self.identity)
            .enforce(&self.root, target, &self.identity)
    }

    /// Get the identity's log_user string for event logging.
    pub fn log_user(&self) -> String {
        self.identity.log_user()
    }
}
