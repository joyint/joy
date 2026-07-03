// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The AI job model (git-native, JOY-01EA). A delegated AI job is a
//! `.joy/ai/jobs/<id>.yaml` file: the current state of one unit of AI work
//! derived from an item. The forge is the source of truth; the audit trail
//! is `.joy/logs`, the history is git, and the work product is a branch on
//! the forge. Money is held in whole cents to avoid floating-point drift.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::member_ref::MemberRef;

/// The kind of work a job carries out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobType {
    /// Repo work: change code to satisfy the item.
    Implement,
    /// Review an existing change or item.
    Review,
    /// Estimate effort/cost.
    Estimate,
    /// Analyze the item or codebase (Joy work, read-only).
    Analyze,
    /// Produce a plan (Joy work).
    Plan,
}

impl std::fmt::Display for JobType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Implement => "implement",
            Self::Review => "review",
            Self::Estimate => "estimate",
            Self::Analyze => "analyze",
            Self::Plan => "plan",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for JobType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "implement" => Ok(Self::Implement),
            "review" => Ok(Self::Review),
            "estimate" => Ok(Self::Estimate),
            "analyze" => Ok(Self::Analyze),
            "plan" => Ok(Self::Plan),
            other => Err(format!("unknown job type: {other}")),
        }
    }
}

/// The lifecycle of a job. The gate holds at `AwaitingApproval`; a human
/// either requests changes (back to `ChangesRequested` -> the runner
/// resumes) or approves (`Done`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobStatus {
    Queued,
    Running,
    AwaitingApproval,
    ChangesRequested,
    Done,
    Failed,
    Cancelled,
}

impl JobStatus {
    /// Whether this is a terminal state (no further work happens).
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::AwaitingApproval => "awaiting-approval",
            Self::ChangesRequested => "changes-requested",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for JobStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "awaiting-approval" => Ok(Self::AwaitingApproval),
            "changes-requested" => Ok(Self::ChangesRequested),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(format!("unknown job status: {other}")),
        }
    }
}

/// A spending cap. Money in whole cents; currency is an ISO code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    pub max_cents: u64,
    #[serde(default = "default_currency")]
    pub currency: String,
}

fn default_currency() -> String {
    "EUR".to_string()
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_cents: 0,
            currency: default_currency(),
        }
    }
}

/// Actual spend so far. Money in whole cents.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cost {
    #[serde(default)]
    pub spent_cents: u64,
    #[serde(default)]
    pub tokens: u64,
}

/// A human review of a job: the gate outcome plus optional feedback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewDecision {
    RequestChanges,
    Approve,
}

/// One round of human review recorded on the job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRound {
    pub at: DateTime<Utc>,
    pub by: MemberRef,
    pub decision: ReviewDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
}

/// A delegated AI job: the current state of one unit of AI work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    /// Short, stable id (also the file stem).
    pub id: String,
    /// The item this job derives from.
    pub item: String,
    #[serde(rename = "type")]
    pub job_type: JobType,
    /// The AI member doing the work (e.g. ai:claude@joy).
    pub actor: MemberRef,
    /// The human who delegated it.
    pub delegated_by: MemberRef,
    pub status: JobStatus,
    /// The branch the AI works on and pushes (repo work only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default)]
    pub budget: Budget,
    #[serde(default)]
    pub cost: Cost,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    /// A one-line human-readable outcome/summary of where the job stands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// Set when the job failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The human review rounds, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reviews: Vec<ReviewRound>,
}

impl Job {
    /// A fresh queued job.
    pub fn new(
        id: impl Into<String>,
        item: impl Into<String>,
        job_type: JobType,
        actor: MemberRef,
        delegated_by: MemberRef,
        budget: Budget,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            id: id.into(),
            item: item.into(),
            job_type,
            actor,
            delegated_by,
            status: JobStatus::Queued,
            branch: None,
            budget,
            cost: Cost::default(),
            created: now,
            updated: now,
            result: None,
            error: None,
            reviews: Vec::new(),
        }
    }
}
