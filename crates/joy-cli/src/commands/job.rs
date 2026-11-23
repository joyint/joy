// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::{bail, Result};
use chrono::Utc;
use clap::{Args, Subcommand};

use joy_core::auth::job_token;
use joy_core::guard::Action;
use joy_core::items;
use joy_core::model::item::{Comment, ItemType, Status};

use crate::color;

#[derive(Args)]
#[command(after_long_help = "\
A job of an AI member runs on a platform, with a token the person who
opened it issued for that platform. `joy job revoke` takes the job back:
its token and its platform are removed and it is new again. The platform
ends its round when it next fetches, keeps the result branch and says so
in a comment on the job.

Examples:
  joy job revoke IT-JOB-0001-AA")]
pub struct JobArgs {
    #[command(subcommand)]
    command: JobCommand,
}

#[derive(Subcommand)]
enum JobCommand {
    /// Take a job back from the platform that runs it
    Revoke(RevokeArgs),
}

#[derive(Args)]
struct RevokeArgs {
    /// Job ID (e.g. IT-JOB-0001-AA)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    id: String,
}

pub fn run(args: JobArgs) -> Result<()> {
    match args.command {
        JobCommand::Revoke(a) => revoke(a),
    }
}

fn revoke(args: RevokeArgs) -> Result<()> {
    let mut ctx = crate::crypt_session::load_context()?;
    let mut job = items::load_item(&ctx.root, &args.id)?;
    if !matches!(job.item_type, ItemType::Job) {
        bail!("{} is not a job", job.id);
    }
    let Some(platform) = job_token::runs_on(&job).map(str::to_string) else {
        bail!("{} runs on no platform", job.id);
    };
    let from = job.status.clone();
    crate::auth_gate::enforce(
        &mut ctx,
        &Action::ChangeJobStatus {
            from: from.clone(),
            to: Status::New,
            assignee: job.assignees.first().map(|a| a.member.clone()),
        },
        &job.id,
    )?;

    let log_user = ctx.log_user();
    job_token::revoke(&mut job);
    job.status = Status::New;
    job.comments.push(Comment {
        author: log_user.clone().into(),
        date: Utc::now(),
        text: format!("Taken back from {platform}."),
        edits: Vec::new(),
    });
    items::touch_for_attribute_change(&mut job, &log_user);
    items::update_item(&ctx.root, &job)?;
    joy_core::event_log::log_event_as(
        &ctx.root,
        joy_core::event_log::EventType::JobStatusChanged,
        &job.id,
        Some(&format!("{from} -> new: taken back from {platform}")),
        &log_user,
    );
    if crate::output::is_json() {
        return crate::output::emit(&job);
    }
    println!(
        "{} taken back from {}, {} -> {}",
        color::id(&job.id),
        color::user(&platform),
        color::status(&from),
        color::status(&Status::New)
    );
    Ok(())
}
