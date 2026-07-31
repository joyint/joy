// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! 2026-07 legacy AI job record cleanup (JOY-0207-DC).
//!
//! Jobs became first-class items in `.joy/jobs/` (JOY-01FE-37); the
//! parallel `.joy/ai/jobs/<id>.yaml` record model is retired. This
//! reconcile deletes a leftover `.joy/ai/jobs/` directory and, when
//! `.joy/ai/` is empty afterwards, that directory too. The legacy
//! `.joy/ai/agents/` store is removed by the sibling
//! `m_2026_07_remove_ai_agents` migration.
//!
//! One-shot, filesystem-aware, idempotent: a repo without `.joy/ai/jobs`
//! is a no-op. Remove this module and its entry in `repo::apply` /
//! `repo::pending` after the deprecation window.

use std::path::Path;

use super::Reconciled;
use crate::error::JoyError;
use crate::store;

const KEY: &str = ".joy/ai/jobs";
const TO: &str = "removed (jobs are items in .joy/jobs/, JOY-01FE-37)";

/// Read-only: what this migration would remove for the project at `root`.
pub fn pending(root: &Path) -> Result<Vec<Reconciled>, JoyError> {
    let jobs_dir = store::joy_dir(root).join(store::AI_JOBS_DIR);
    if jobs_dir.is_dir() {
        Ok(vec![Reconciled { key: KEY, to: TO }])
    } else {
        Ok(Vec::new())
    }
}

/// Delete `.joy/ai/jobs/` (and an empty `.joy/ai/` shell) at `root`.
pub fn migrate(root: &Path) -> Result<Vec<Reconciled>, JoyError> {
    let jobs_dir = store::joy_dir(root).join(store::AI_JOBS_DIR);
    if !jobs_dir.is_dir() {
        return Ok(Vec::new());
    }
    std::fs::remove_dir_all(&jobs_dir).map_err(|e| JoyError::WriteFile {
        path: jobs_dir.clone(),
        source: e,
    })?;
    let ai_dir = store::joy_dir(root).join(store::AI_DIR);
    let ai_empty = std::fs::read_dir(&ai_dir)
        .map(|mut d| d.next().is_none())
        .unwrap_or(false);
    if ai_empty {
        let _ = std::fs::remove_dir(&ai_dir);
    }
    // Stage the deletions so the next commit records the cleanup.
    let rel_jobs = format!("{}/{}", store::JOY_DIR, store::AI_JOBS_DIR);
    let rel_ai = format!("{}/{}", store::JOY_DIR, store::AI_DIR);
    crate::git_ops::auto_git_add(root, &[&rel_jobs, &rel_ai]);
    Ok(vec![Reconciled { key: KEY, to: TO }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn setup(root: &Path) {
        fs::create_dir_all(store::joy_dir(root)).unwrap();
    }

    #[test]
    fn removes_jobs_dir_and_empty_ai_shell() {
        let dir = tempdir().unwrap();
        setup(dir.path());
        let jobs = store::joy_dir(dir.path()).join(store::AI_JOBS_DIR);
        fs::create_dir_all(&jobs).unwrap();
        fs::write(jobs.join("abc123.yaml"), "id: abc123\n").unwrap();

        let done = migrate(dir.path()).unwrap();
        assert_eq!(done.len(), 1);
        assert!(!jobs.exists());
        assert!(!store::joy_dir(dir.path()).join(store::AI_DIR).exists());
    }

    #[test]
    fn keeps_ai_dir_when_agents_exist() {
        let dir = tempdir().unwrap();
        setup(dir.path());
        let jobs = store::joy_dir(dir.path()).join(store::AI_JOBS_DIR);
        let agents = store::joy_dir(dir.path()).join(store::AI_AGENTS_DIR);
        fs::create_dir_all(&jobs).unwrap();
        fs::create_dir_all(&agents).unwrap();
        fs::write(agents.join("claude.yaml"), "adapter: acp\n").unwrap();

        migrate(dir.path()).unwrap();
        assert!(!jobs.exists());
        assert!(agents.join("claude.yaml").is_file());
    }

    #[test]
    fn no_op_without_legacy_dir() {
        let dir = tempdir().unwrap();
        setup(dir.path());
        assert!(pending(dir.path()).unwrap().is_empty());
        assert!(migrate(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn pending_reports_without_removing() {
        let dir = tempdir().unwrap();
        setup(dir.path());
        let jobs = store::joy_dir(dir.path()).join(store::AI_JOBS_DIR);
        fs::create_dir_all(&jobs).unwrap();
        let p = pending(dir.path()).unwrap();
        assert_eq!(p.len(), 1);
        assert!(jobs.exists());
    }
}
