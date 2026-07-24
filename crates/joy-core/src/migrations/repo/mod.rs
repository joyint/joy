// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Repo-layout migrations applied at sync time (`joy update` / auto-sync).
//!
//! Unlike `project_yaml` schema migrations -- pure on-read transforms of
//! the parsed YAML value -- these are filesystem-aware, one-shot
//! reconciles: they may inspect files on disk and rewrite project.yaml.
//! Each lives in a date-prefixed `m_<yyyy_mm>_<slug>.rs` module and is
//! removable in one step after its deprecation window: delete the module
//! file and its entry in [`apply`] / [`pending`].

mod m_2026_06_doc_path_layout;
mod m_2026_07_mode_to_interaction;
mod m_2026_07_remove_ai_agents;
mod m_2026_07_remove_ai_jobs;
mod m_2026_07_three_levels;

use std::path::Path;

use crate::error::JoyError;

/// A single reconcile a repo migration applied, surfaced in the
/// `joy update` output.
pub struct Reconciled {
    /// The project.yaml `docs.*` key (or analogous field) that was set.
    pub key: &'static str,
    /// The value it was pinned to (a path that exists on disk).
    pub to: &'static str,
}

/// Read-only: the reconciles the repo migrations would apply at `root`.
pub fn pending(root: &Path) -> Result<Vec<Reconciled>, JoyError> {
    let mut out = m_2026_06_doc_path_layout::pending(root)?;
    out.extend(m_2026_07_remove_ai_jobs::pending(root)?);
    out.extend(m_2026_07_remove_ai_agents::pending(root)?);
    out.extend(m_2026_07_mode_to_interaction::pending(root)?);
    out.extend(m_2026_07_three_levels::pending(root)?);
    Ok(out)
}

/// Apply every repo migration in order. Returns the reconciles performed.
pub fn apply(root: &Path) -> Result<Vec<Reconciled>, JoyError> {
    let mut out = m_2026_06_doc_path_layout::migrate(root)?;
    out.extend(m_2026_07_remove_ai_jobs::migrate(root)?);
    out.extend(m_2026_07_remove_ai_agents::migrate(root)?);
    out.extend(m_2026_07_mode_to_interaction::migrate(root)?);
    out.extend(m_2026_07_three_levels::migrate(root)?);
    Ok(out)
}
