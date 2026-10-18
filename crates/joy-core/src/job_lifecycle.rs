// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The job lifecycle (JOY-02D9-0F): the one place that says where a job
//! stands and what happens to it next, for every host alike.
//!
//! A job is an item (JOY-01FE-37) and walks the item's statuses. What a
//! person does with it is the item machinery's. What the PLATFORM does
//! with it, and what every surface shows about it, is decided here and
//! nowhere else (operator 2026-10-16, after a job went to review over a
//! missing API key and nobody could say why):
//!
//! | where it stands                      | the platform             | the signal            |
//! |--------------------------------------|--------------------------|-----------------------|
//! | new, and it cannot start             | refuses `new -> open`    | blocked, the reason   |
//! | open, and it cannot start            | sends it back to new     | blocked, the reason   |
//! | open, and it can                     | its assignee takes it    | normal                |
//! | in-progress                          | a round runs             | normal                |
//! | a round ended, the AI asked          | review                   | question              |
//! | a round ended with its work          | review                   | delivered             |
//! | a round broke                        | review                   | failed, the error     |
//! | a round was stopped by a person      | stays as the person set  | normal                |
//! | review, the answer is in             | the round resumes        | normal                |
//!
//! Review is the one place where a person is needed; new with the
//! blocked signal means "not set up yet". The dialog between the person
//! and the AI runs over the job's comments, in both directions.
//!
//! Nothing here reads a database or the network. What only a host knows
//! (whether the assignee's agent can run here: a key, an adapter) comes
//! in as the host's sentence and goes out as the reason, unchanged.

use crate::migrations::ai_member_name;
use crate::model::item::{ActivityOutcome, Item, JobFeedback, Status};
use crate::model::project::Project;
use joy_model::InteractionLevel;

/// What a surface shows about a job next to its status: nothing special,
/// or the one thing a person has to know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    /// Nothing to say.
    Normal,
    /// The job cannot start, and why. Only in new and open.
    Blocked(String),
    /// The last round broke, and how. Only in review.
    Failed(String),
    /// The AI asked something in the job's comments and waits.
    Question,
    /// The last round ended with its work, nothing asked, nothing broke.
    Delivered,
}

impl Signal {
    /// The one word a wire or a class name carries.
    pub fn kind(&self) -> &'static str {
        match self {
            Signal::Normal => "normal",
            Signal::Blocked(_) => "blocked",
            Signal::Failed(_) => "failed",
            Signal::Question => "question",
            Signal::Delivered => "delivered",
        }
    }

    /// What the signal says, where it says something.
    pub fn text(&self) -> &str {
        match self {
            Signal::Blocked(text) | Signal::Failed(text) => text,
            _ => "",
        }
    }
}

/// The host's sentence for an AI member whose agent has no key here.
pub fn no_key(assignee: &str) -> String {
    format!(
        "{} has no API key; set it in Settings",
        ai_member_name::read(assignee)
    )
}

/// The host's sentence for an AI member whose agent is not set up here.
pub fn no_agent(assignee: &str) -> String {
    format!(
        "{} has no agent set up; set it in Settings",
        ai_member_name::read(assignee)
    )
}

/// What the job's rounds have cost so far.
pub fn spent_cents(job: &Item) -> u64 {
    job.job
        .as_ref()
        .map(|s| s.activity.iter().map(|a| a.cost_cents).sum())
        .unwrap_or(0)
}

/// The cap a job runs under (JP-012F-B5): its own budget when it has
/// one, 0 included (a real zero: a model that costs nothing may run, a
/// paid one stops at its first cost); the project default when it has
/// none of its own. None for an item that is not a job.
pub fn budget_cap(job: &Item, default_cents: u64) -> Option<u64> {
    let spec = job.job.as_ref()?;
    Some(
        spec.budget
            .as_ref()
            .and_then(|b| b.max_cents)
            .unwrap_or(default_cents),
    )
}

/// Why a round must not start, if so: money has flowed and none is
/// left. A fresh job under a zero cap may start (Horst 2026-09-09: free
/// models); it is the recorded spend reaching the cap that ends the
/// job's rounds.
pub fn budget_exceeded(job: &Item, default_cents: u64) -> Option<String> {
    let max = budget_cap(job, default_cents)?;
    let spent = spent_cents(job);
    (spent > 0 && spent >= max).then(|| {
        format!(
            "job {} has spent {spent} of {max} cents; raise the budget to run another round",
            job.id
        )
    })
}

/// Cents left under the job's cap: what a round may still spend.
/// Saturating: an overspent job reports 0. None for an item that is
/// not a job.
pub fn remaining_budget_cents(job: &Item, default_cents: u64) -> Option<u64> {
    let max = budget_cap(job, default_cents)?;
    Some(max.saturating_sub(spent_cents(job)))
}

/// Why the job cannot start now, if so, for the person `person_key`
/// (who approved it, or who looks at it): no assignee yet; what the
/// host says about the assignee's agent (`host_block`); the budget used
/// up; and, while the job is still new, the assignee without the jobs
/// capability for the person or the job asking for a level it may not
/// run at for them (from the approval on the level is the job's own,
/// JI-0166-D8). A job assigned to a person is never blocked here: they
/// start it themselves.
pub fn start_block(
    project: &Project,
    job: &Item,
    person_key: &str,
    default_budget_cents: u64,
    host_block: Option<&str>,
) -> Option<String> {
    let Some(assignee) = job.assignees.first().map(|a| a.member.id().to_string()) else {
        return Some("the job has no assignee yet".to_string());
    };
    if !project.is_ai(&assignee) {
        return None;
    }
    if let Some(block) = host_block {
        return Some(block.to_string());
    }
    if let Some(reason) = budget_exceeded(job, default_budget_cents) {
        return Some(reason);
    }
    if job.status != Status::New {
        return None;
    }
    crate::auth::grants::job_terms(project, job, person_key).err()
}

/// Where a job goes when a round of it ends with `outcome`: to review,
/// whatever the round brought (its work, a question in the comments, or
/// an error), because that is where a person is needed. A round a
/// person stopped leaves the status as the person set it.
pub fn after_round(outcome: ActivityOutcome) -> Option<Status> {
    match outcome {
        ActivityOutcome::Succeeded | ActivityOutcome::Failed => Some(Status::Review),
        ActivityOutcome::Aborted => None,
    }
}

/// The signal of a job, with `block` as [`start_block`] found it (or
/// nothing, where the host did not ask).
pub fn signal(job: &Item, block: Option<&str>) -> Signal {
    let spec = job.job.as_ref();
    let feedback = spec.and_then(|s| s.feedback);
    let last = spec.and_then(|s| s.activity.last());
    match job.status {
        Status::New | Status::Open => match block {
            Some(reason) => Signal::Blocked(reason.to_string()),
            None => Signal::Normal,
        },
        Status::InProgress => {
            // a question from before questions went to review (data
            // written by an older platform) still reads as one
            if feedback == Some(JobFeedback::Awaited) {
                Signal::Question
            } else {
                Signal::Normal
            }
        }
        Status::Review => {
            if feedback == Some(JobFeedback::Awaited) {
                return Signal::Question;
            }
            match last.map(|a| &a.outcome) {
                Some(ActivityOutcome::Failed) => {
                    Signal::Failed(last.and_then(|a| a.error.clone()).unwrap_or_default())
                }
                Some(ActivityOutcome::Succeeded) => Signal::Delivered,
                _ => Signal::Normal,
            }
        }
        Status::Closed | Status::Deferred => Signal::Normal,
    }
}

/// Whether the status bar offers nothing: a job in new that cannot
/// start is not sent off, whatever the person's capabilities. A job in
/// review keeps every step, broken round or not: the person decides.
pub fn locked(job: &Item, signal: &Signal) -> bool {
    job.status == Status::New && matches!(signal, Signal::Blocked(_))
}

/// Whether the platform sends an open job back to new: it cannot start
/// (`block`, which [`start_block`] gives only for a job the platform
/// would have to start: one with an AI assignee, or none). Written with
/// the reason in the event log, nothing on the job's activity: a job
/// that never ran stays one that never ran, so its assignee takes it
/// once it is set up.
pub fn bounces(job: &Item, block: Option<&str>) -> bool {
    job.status == Status::Open && block.is_some()
}

/// The event log line of a bounce.
pub fn bounce_event(reason: &str) -> String {
    format!("open -> new: {reason}")
}

/// Whether the platform resumes a round on this job: the answer to its
/// question is in. From review (the question went there) or from
/// in-progress (a job an older platform held there).
pub fn resumes(job: &Item) -> bool {
    matches!(job.status, Status::Review | Status::InProgress)
        && job.job.as_ref().and_then(|s| s.feedback) == Some(JobFeedback::Received)
}

/// The words the platform lends the AI when a tool it asked for is not
/// allowed at the job's level: as the AI would ask, with what the
/// person can do about it. A job runs at proposing or at autonomous; at
/// autonomous a refusal is final.
pub fn escalation_words(tool: &str, level: InteractionLevel) -> String {
    match level {
        InteractionLevel::Autonomous => format!(
            "I want to run `{tool}`, which is not allowed in a job. How should I go on without it?"
        ),
        level => format!(
            "I want to run `{tool}`. This job runs at {level}, and for that I need autonomous. \
             Will you raise the level, or how should I go on without it?"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::item::{ItemType, JobActivity, JobSpec, Priority};
    use chrono::Utc;

    const STATUSES: [Status; 6] = [
        Status::New,
        Status::Open,
        Status::InProgress,
        Status::Review,
        Status::Closed,
        Status::Deferred,
    ];
    const FEEDBACKS: [Option<JobFeedback>; 3] = [
        None,
        Some(JobFeedback::Awaited),
        Some(JobFeedback::Received),
    ];
    const OUTCOMES: [Option<ActivityOutcome>; 4] = [
        None,
        Some(ActivityOutcome::Succeeded),
        Some(ActivityOutcome::Failed),
        Some(ActivityOutcome::Aborted),
    ];

    fn activity(outcome: ActivityOutcome, cost: u64) -> JobActivity {
        JobActivity {
            started: Utc::now(),
            ended: Some(Utc::now()),
            outcome,
            tokens: 0,
            cost_cents: cost,
            branch: None,
            result: None,
            error: Some("the agent died".into()),
            by: None,
            model: None,
        }
    }

    fn job(status: Status, feedback: Option<JobFeedback>, last: Option<ActivityOutcome>) -> Item {
        let mut item = Item::new(
            "J-JOB-1".into(),
            "a job".into(),
            ItemType::Job,
            Priority::Medium,
            Vec::new(),
        );
        item.status = status;
        item.job = Some(JobSpec {
            scope: vec!["J-0001".into()],
            budget: None,
            window: None,
            feedback,
            activity: last.map(|o| vec![activity(o, 0)]).unwrap_or_default(),
            base_branch: None,
            result_branch: None,
        });
        item
    }

    /// Every combination, against the table in the module doc: the
    /// signal says the one thing a person has to know, and nothing else.
    #[test]
    fn the_signal_of_every_combination_follows_the_table() {
        for status in STATUSES {
            for feedback in FEEDBACKS {
                for last in OUTCOMES {
                    for block in [None, Some("vibe has no API key; set it in Settings")] {
                        let j = job(status.clone(), feedback, last);
                        let got = signal(&j, block);
                        let want = match (&status, block, feedback, last) {
                            (Status::New | Status::Open, Some(b), _, _) => {
                                Signal::Blocked(b.to_string())
                            }
                            (Status::New | Status::Open, None, _, _) => Signal::Normal,
                            (Status::InProgress, _, Some(JobFeedback::Awaited), _) => {
                                Signal::Question
                            }
                            (Status::InProgress, _, _, _) => Signal::Normal,
                            (Status::Review, _, Some(JobFeedback::Awaited), _) => Signal::Question,
                            (Status::Review, _, _, Some(ActivityOutcome::Failed)) => {
                                Signal::Failed("the agent died".into())
                            }
                            (Status::Review, _, _, Some(ActivityOutcome::Succeeded)) => {
                                Signal::Delivered
                            }
                            (Status::Review, _, _, _) => Signal::Normal,
                            (Status::Closed | Status::Deferred, _, _, _) => Signal::Normal,
                        };
                        assert_eq!(got, want, "{status} {feedback:?} {last:?} {block:?}");
                        // the bar is locked in new while blocked, nowhere else
                        assert_eq!(
                            locked(&j, &got),
                            status == Status::New && block.is_some(),
                            "{status} {block:?}"
                        );
                    }
                }
            }
        }
    }

    /// A round ends in review whatever it brought; only a person's stop
    /// leaves the status alone.
    #[test]
    fn every_round_that_ends_by_itself_goes_to_review() {
        assert_eq!(
            after_round(ActivityOutcome::Succeeded),
            Some(Status::Review)
        );
        assert_eq!(after_round(ActivityOutcome::Failed), Some(Status::Review));
        assert_eq!(after_round(ActivityOutcome::Aborted), None);
    }

    /// What the platform does by itself, in one step: the watcher's
    /// bounce and resume, the runner's round end. `next` is what the
    /// next round would bring.
    fn platform_step(j: &mut Item, block: Option<&str>, next: ActivityOutcome) -> bool {
        if bounces(j, block) {
            j.status = Status::New;
            return true;
        }
        if resumes(j) {
            j.job.as_mut().unwrap().feedback = None;
            j.status = Status::InProgress;
            return true;
        }
        if j.status == Status::Open && block.is_none() {
            // the assignee takes it
            j.status = Status::InProgress;
            return true;
        }
        if j.status == Status::InProgress {
            j.job.as_mut().unwrap().activity.push(activity(next, 0));
            if let Some(to) = after_round(next) {
                j.status = to;
                return true;
            }
            // stopped by a person: the person set the status
            j.status = Status::Open;
            return false;
        }
        false
    }

    /// Left alone, every job comes to rest within a few steps: no job
    /// goes open, new, open on its own, and no answered question runs a
    /// round twice. From every start, with and without a block, and
    /// whatever the next round brings.
    #[test]
    fn left_alone_every_job_settles() {
        for status in STATUSES {
            for feedback in FEEDBACKS {
                for last in OUTCOMES {
                    for block in [None, Some("no key")] {
                        for next in [ActivityOutcome::Succeeded, ActivityOutcome::Failed] {
                            let mut j = job(status.clone(), feedback, last);
                            let mut steps = 0;
                            while platform_step(&mut j, block, next) {
                                steps += 1;
                                assert!(
                                    steps <= 4,
                                    "{status} {feedback:?} {last:?} {block:?}: never settles"
                                );
                            }
                            // and once at rest, the platform has nothing to do
                            let before = (j.status.clone(), j.job.as_ref().unwrap().feedback);
                            assert!(!platform_step(&mut j, block, next));
                            assert_eq!(
                                before,
                                (j.status.clone(), j.job.as_ref().unwrap().feedback)
                            );
                            // at rest, a job is never open with a block,
                            // and never in-progress with an answer waiting
                            assert!(!(j.status == Status::Open && block.is_some()));
                            assert!(!resumes(&j));
                        }
                    }
                }
            }
        }
    }

    /// The budget: a fresh job under a zero cap may start, a job that
    /// spent its cap may not.
    #[test]
    fn the_budget_ends_a_jobs_rounds_once_money_flowed() {
        let fresh = job(Status::Open, None, None);
        assert_eq!(budget_cap(&fresh, 500), Some(500));
        assert!(budget_exceeded(&fresh, 0).is_none(), "a free model may run");
        let mut spent = job(Status::Open, None, None);
        spent.job.as_mut().unwrap().activity = vec![activity(ActivityOutcome::Succeeded, 500)];
        assert!(budget_exceeded(&spent, 500).is_some());
        assert_eq!(remaining_budget_cents(&spent, 500), Some(0));
        assert_eq!(remaining_budget_cents(&spent, 900), Some(400));
        let mut not_a_job = job(Status::Open, None, None);
        not_a_job.job = None;
        assert_eq!(budget_cap(&not_a_job, 500), None);
    }

    /// The words of an escalation name the tool, the level and what the
    /// person can do; at autonomous nothing can be raised.
    #[test]
    fn the_escalation_speaks_as_the_ai_would() {
        let asked = escalation_words("git commit", InteractionLevel::Proposing);
        assert!(asked.starts_with("I want to run `git commit`."), "{asked}");
        assert!(asked.contains("runs at proposing"), "{asked}");
        assert!(asked.contains("raise the level"), "{asked}");
        let final_no = escalation_words("rm -rf build", InteractionLevel::Autonomous);
        assert!(final_no.contains("not allowed in a job"), "{final_no}");
        assert!(!final_no.contains("raise"), "{final_no}");
    }
}
