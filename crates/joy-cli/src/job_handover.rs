// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The token a person gives a job when they hand it to its AI assignee
//! (JI-01A7-0F): what `joy status`, its shortcuts and `joy edit
//! --feedback received` do with the job before they write it. The rules
//! are joy-core's ([`joy_core::auth::job_token`]); here is the person's
//! key, which this host can ask for.

use std::path::Path;

use anyhow::Result;

use joy_core::auth::{job_token, platform};
use joy_core::identity::Identity;
use joy_core::model::item::{Item, Status};
use joy_core::store;

/// Bring the job's token in line with the step that took it from `from`
/// to where `job` stands now: a fresh one, sealed for its platform, when
/// `by` hands it to its AI assignee; none on a job that is new, closed
/// or deferred. `named` is the platform the person named for it.
pub fn step(
    root: &Path,
    by: &Identity,
    job: &mut Item,
    from: &Status,
    named: Option<&str>,
) -> Result<()> {
    if matches!(job.status, Status::New | Status::Closed | Status::Deferred) {
        job_token::clear(job);
    }
    let project = store::load_project(root)?;
    if !job_token::handed_over(&project, job, from, by) {
        if named.is_some() {
            anyhow::bail!("--platform names where a job runs when it is opened");
        }
        return Ok(());
    }
    platform::fits(&project).map_err(|why| anyhow::anyhow!(why))?;
    // Where a job runs is fixed when it is opened; a job already on a
    // platform changes hands only by being taken back first.
    let fixed = (*from != Status::New)
        .then(|| job_token::runs_on(job))
        .flatten();
    if let (Some(fixed), Some(named)) = (fixed, named) {
        if fixed != named {
            anyhow::bail!(
                "job {id} runs on {fixed}. Take it back first: joy job revoke {id}",
                id = job.id
            );
        }
    }
    let domain = platform::for_job(&project, job_token::runs_on(job), named)
        .map_err(|why| anyhow::anyhow!(why))?;
    let operator = crate::auth_gate::unlock(root, &project, by.member.id())?;
    let token = job_token::issue(&project, job, &operator.member_key, &operator.seed, &domain)
        .map_err(|why| anyhow::anyhow!(why))?;
    job_token::put(job, token);
    Ok(())
}
