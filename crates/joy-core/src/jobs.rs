// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Read/write for git-native AI job records under `.joy/ai/jobs/` (JOY-01EA).
//! Mirrors the item store: one YAML file per job, staged into git on save so
//! the forge stays the source of truth. Jobs are not encrypted in this
//! build-out; per-zone encryption is a later additive layer.

use std::path::Path;

use crate::error::JoyError;
use crate::model::job::Job;
use crate::store;

fn jobs_dir(root: &Path) -> std::path::PathBuf {
    store::joy_dir(root).join(store::AI_JOBS_DIR)
}

/// A fresh, short, file-safe job id.
pub fn new_job_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

/// Save a job to `.joy/ai/jobs/<id>.yaml` and stage it.
pub fn save_job(root: &Path, job: &Job) -> Result<(), JoyError> {
    let dir = jobs_dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| JoyError::CreateDir {
        path: dir.clone(),
        source: e,
    })?;
    let filename = format!("{}.yaml", job.id);
    store::write_yaml(&dir.join(&filename), job)?;
    let rel = format!("{}/{}/{}", store::JOY_DIR, store::AI_JOBS_DIR, filename);
    crate::git_ops::auto_git_add(root, &[&rel]);
    Ok(())
}

/// Load one job by id, if present.
pub fn load_job(root: &Path, id: &str) -> Result<Option<Job>, JoyError> {
    let path = jobs_dir(root).join(format!("{id}.yaml"));
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(store::read_yaml(&path)?))
}

/// Load every job, newest first (by `created`).
pub fn load_jobs(root: &Path) -> Result<Vec<Job>, JoyError> {
    let dir = jobs_dir(root);
    let mut jobs = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(jobs);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        jobs.push(store::read_yaml::<Job>(&path)?);
    }
    jobs.sort_by_key(|j| std::cmp::Reverse(j.created));
    Ok(jobs)
}

/// Jobs derived from a given item, newest first.
pub fn jobs_for_item(root: &Path, item_id: &str) -> Result<Vec<Job>, JoyError> {
    Ok(load_jobs(root)?
        .into_iter()
        .filter(|j| j.item == item_id)
        .collect())
}

// ---- lifecycle (shared by the CLI, the desktop, and the platform) -------

use crate::event_log::{log_event_as, EventType};
use crate::member_ref::MemberRef;
use crate::model::job::{Budget, JobStatus, JobType, ReviewDecision, ReviewRound};
use chrono::{DateTime, Utc};

/// Delegate a new job: create the queued record, save it, and audit it.
/// `actor_log` is the acting identity's log string (may carry delegated-by).
#[allow(clippy::too_many_arguments)]
pub fn delegate(
    root: &Path,
    item: &str,
    job_type: JobType,
    actor: MemberRef,
    delegated_by: MemberRef,
    budget: Budget,
    actor_log: &str,
    now: DateTime<Utc>,
) -> Result<Job, JoyError> {
    let job = Job::new(
        new_job_id(),
        item,
        job_type,
        actor,
        delegated_by,
        budget,
        now,
    );
    save_job(root, &job)?;
    log_event_as(
        root,
        EventType::JobDelegated,
        &job.id,
        Some(&format!("{} on {}", job.job_type, job.item)),
        actor_log,
    );
    Ok(job)
}

/// Move a job to a new status, persist, and audit.
pub fn transition(
    root: &Path,
    job: &mut Job,
    status: JobStatus,
    actor_log: &str,
    now: DateTime<Utc>,
) -> Result<(), JoyError> {
    job.status = status;
    job.updated = now;
    save_job(root, job)?;
    log_event_as(
        root,
        EventType::JobStatusChanged,
        &job.id,
        Some(&status.to_string()),
        actor_log,
    );
    Ok(())
}

/// Record a human decision to request changes: append a review round,
/// set the status to changes-requested (the runner resumes), persist, audit.
pub fn request_changes(
    root: &Path,
    job: &mut Job,
    by: MemberRef,
    feedback: Option<String>,
    actor_log: &str,
    now: DateTime<Utc>,
) -> Result<(), JoyError> {
    job.reviews.push(ReviewRound {
        at: now,
        by,
        decision: ReviewDecision::RequestChanges,
        feedback,
    });
    job.status = JobStatus::ChangesRequested;
    job.updated = now;
    save_job(root, job)?;
    log_event_as(
        root,
        EventType::JobReviewed,
        &job.id,
        Some("request-changes"),
        actor_log,
    );
    Ok(())
}

/// Record a human approval: append a review round and mark the job done.
pub fn approve(
    root: &Path,
    job: &mut Job,
    by: MemberRef,
    actor_log: &str,
    now: DateTime<Utc>,
) -> Result<(), JoyError> {
    job.reviews.push(ReviewRound {
        at: now,
        by,
        decision: ReviewDecision::Approve,
        feedback: None,
    });
    job.status = JobStatus::Done;
    job.updated = now;
    save_job(root, job)?;
    log_event_as(
        root,
        EventType::JobReviewed,
        &job.id,
        Some("approve"),
        actor_log,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn ts() -> DateTime<Utc> {
        "2026-07-04T00:00:00Z".parse().unwrap()
    }

    fn actor() -> MemberRef {
        MemberRef::new("ai:claude@joy")
    }
    fn human() -> MemberRef {
        MemberRef::new("horst@example.com")
    }

    #[test]
    fn delegate_save_load_roundtrip() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".joy/logs")).unwrap();
        let job = delegate(
            dir.path(),
            "LP-0001",
            JobType::Implement,
            actor(),
            human(),
            Budget {
                max_cents: 500,
                currency: "EUR".into(),
            },
            "ai:claude@joy delegated-by:horst@example.com",
            ts(),
        )
        .unwrap();
        assert_eq!(job.status, JobStatus::Queued);

        let loaded = load_job(dir.path(), &job.id).unwrap().unwrap();
        assert_eq!(loaded, job);
        assert_eq!(load_jobs(dir.path()).unwrap().len(), 1);
        assert_eq!(jobs_for_item(dir.path(), "LP-0001").unwrap().len(), 1);
        assert_eq!(jobs_for_item(dir.path(), "OTHER").unwrap().len(), 0);
    }

    #[test]
    fn full_lifecycle_transitions() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".joy/logs")).unwrap();
        let mut job = delegate(
            dir.path(),
            "LP-0002",
            JobType::Implement,
            actor(),
            human(),
            Budget::default(),
            "log",
            ts(),
        )
        .unwrap();

        transition(dir.path(), &mut job, JobStatus::Running, "log", ts()).unwrap();
        transition(
            dir.path(),
            &mut job,
            JobStatus::AwaitingApproval,
            "log",
            ts(),
        )
        .unwrap();
        assert_eq!(
            load_job(dir.path(), &job.id).unwrap().unwrap().status,
            JobStatus::AwaitingApproval
        );

        // request changes -> resumes -> awaiting again -> approve -> done
        request_changes(
            dir.path(),
            &mut job,
            human(),
            Some("tighten it".into()),
            "log",
            ts(),
        )
        .unwrap();
        assert_eq!(job.status, JobStatus::ChangesRequested);
        assert_eq!(job.reviews.len(), 1);
        transition(dir.path(), &mut job, JobStatus::Running, "log", ts()).unwrap();
        transition(
            dir.path(),
            &mut job,
            JobStatus::AwaitingApproval,
            "log",
            ts(),
        )
        .unwrap();
        approve(dir.path(), &mut job, human(), "log", ts()).unwrap();

        let done = load_job(dir.path(), &job.id).unwrap().unwrap();
        assert_eq!(done.status, JobStatus::Done);
        assert!(done.status.is_terminal());
        assert_eq!(done.reviews.len(), 2);
        assert_eq!(done.reviews[1].decision, ReviewDecision::Approve);
    }
}
