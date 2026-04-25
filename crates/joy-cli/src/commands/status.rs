// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::io::Write;

use anyhow::Result;
use chrono::Utc;
use clap::Args;

use joy_core::context::Context;
use joy_core::guard::Action;
use joy_core::items;
use joy_core::model::item::{Assignee, Status};
use joy_core::releases;
use joy_core::store;

use crate::color;

#[derive(Args)]
#[command(after_help = "\
Workflow:
  new -> open -> in-progress -> review -> closed
                   \\                |
                    +-> deferred <--+

  All transitions are allowed by default. Joy warns but does not block.
  Shortcuts: joy start (in-progress), joy submit (review), joy close (closed).

Behavior:
  - Closing an item with open children prints a warning
  - Starting an item with open dependencies prints a warning
  - When all children of a parent are closed, the parent auto-closes
  - Custom rules can restrict transitions (see joy tutorial, Mission 9)")]
pub struct StatusArgs {
    /// Item ID (e.g. IT-0001)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    id: String,

    /// New status: new, open, in-progress, review, closed, deferred
    status: String,
}

impl StatusArgs {
    pub fn new(id: String, status: String) -> Self {
        Self { id, status }
    }
}

pub fn run(args: StatusArgs) -> Result<()> {
    let ctx = Context::load()?;

    let new_status: Status = args
        .status
        .parse()
        .map_err(|e: String| anyhow::anyhow!("{}", e))?;

    let mut item = items::load_item(&ctx.root, &args.id)?;
    let old_status = item.status.clone();

    ctx.enforce(
        &Action::ChangeStatus {
            from: old_status.clone(),
            to: new_status.clone(),
        },
        &item.id,
    )?;

    // Warn when reopening a released item
    if matches!(old_status, Status::Closed | Status::Deferred)
        && !matches!(new_status, Status::Closed | Status::Deferred)
    {
        if let Ok(Some(release_version)) = releases::item_in_release(&ctx.root, &item.id) {
            eprintln!(
                "\nwarning: {} is included in release {}",
                color::id(&item.id),
                release_version
            );
            eprintln!("  |");
            eprintln!("  = note: reopening a released item means the fix was incomplete");
            eprintln!("  = help: consider creating a new bug item instead");
            eprint!("\n  Reopen anyway? [y/N] ");
            std::io::stderr().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let trimmed = input.trim();
            if !trimmed.eq_ignore_ascii_case("y") {
                println!("Aborted.");
                return Ok(());
            }
        }
    }

    // Warn when closing an item that has open children
    if matches!(new_status, Status::Closed) {
        let all_items = items::load_items(&ctx.root)?;
        let open_children: Vec<_> = all_items
            .iter()
            .filter(|i| i.parent.as_deref() == Some(&item.id) && i.is_active())
            .collect();
        if !open_children.is_empty() {
            eprintln!(
                "Warning: {} has {}:",
                color::id(&item.id),
                color::plural(open_children.len(), "open child item")
            );
            for child in &open_children {
                eprintln!(
                    "  {} {} [{}]",
                    color::id(&child.id),
                    child.title,
                    color::status(&child.status)
                );
            }
        }
    }

    // Warn when starting an item with open dependencies
    if matches!(new_status, Status::InProgress) {
        let all_items = items::load_items(&ctx.root)?;
        let open_deps: Vec<_> = all_items
            .iter()
            .filter(|i| item.deps.contains(&i.id) && i.is_active())
            .collect();
        if !open_deps.is_empty() {
            eprintln!(
                "Warning: {} has {} open dependency(ies):",
                color::id(&item.id),
                open_deps.len()
            );
            for dep in &open_deps {
                eprintln!(
                    "  {} {} [{}]",
                    color::id(&dep.id),
                    dep.title,
                    color::status(&dep.status)
                );
            }
        }
    }

    // Auto-assign on start if no assignees
    if matches!(new_status, Status::InProgress) && item.assignees.is_empty() {
        let config = store::load_config();
        if config.workflow.auto_assign {
            item.assignees.push(Assignee {
                member: ctx.identity.member.clone(),
                capabilities: Vec::new(),
            });
            eprintln!(
                "Auto-assigned {} to {}",
                color::id(&item.id),
                ctx.identity.member
            );

            // Warn if member lacks item capabilities
            let project_path = store::joy_dir(&ctx.root).join(store::PROJECT_FILE);
            if let Ok(project) = store::read_project(&project_path) {
                if let Some(member) = project.members.get(&ctx.identity.member) {
                    if !matches!(
                        member.capabilities,
                        joy_core::model::project::MemberCapabilities::All
                    ) {
                        if let joy_core::model::project::MemberCapabilities::Specific(ref caps) =
                            member.capabilities
                        {
                            for item_cap in &item.capabilities {
                                if !caps.contains_key(item_cap) {
                                    eprintln!(
                                        "Warning: {} does not have capability '{}'",
                                        ctx.identity.member, item_cap
                                    );
                                }
                            }
                        }
                    }
                }
            }
        } else {
            anyhow::bail!(
                "no assignee on {}. Assign first:\n  joy assign {} <MEMBER>",
                item.id,
                item.id
            );
        }
    }

    item.status = new_status.clone();
    item.updated = Utc::now();
    items::update_item(&ctx.root, &item)?;

    let log_user = ctx.log_user();
    joy_core::event_log::log_event_as(
        &ctx.root,
        joy_core::event_log::EventType::ItemStatusChanged,
        &item.id,
        Some(&format!("{old_status} -> {new_status}")),
        &log_user,
    );

    println!(
        "{} {} -> {}",
        color::id(&item.id),
        color::status(&old_status),
        color::status(&new_status)
    );

    // Auto-close parent when all children are closed
    // (must run before auto_git_post_command so auto-close changes are included)
    if let (Status::Closed, Some(ref parent_id)) = (&new_status, &item.parent) {
        let all_items = items::load_items(&ctx.root)?;
        let has_open_siblings = all_items
            .iter()
            .any(|i| i.parent.as_deref() == Some(parent_id) && i.is_active());

        if !has_open_siblings {
            if let Ok(mut parent) = items::load_item(&ctx.root, parent_id) {
                if parent.is_active() {
                    let parent_old = parent.status.clone();
                    parent.status = Status::Closed;
                    parent.updated = Utc::now();
                    items::update_item(&ctx.root, &parent)?;
                    joy_core::event_log::log_event_as(
                        &ctx.root,
                        joy_core::event_log::EventType::ItemStatusChanged,
                        &parent.id,
                        Some(&format!("{parent_old} -> closed (all children closed)")),
                        &log_user,
                    );
                    println!(
                        "{} {} -> {} (all children closed)",
                        color::id(&parent.id),
                        color::status(&parent_old),
                        color::status(&parent.status)
                    );
                }
            }
        }
    }

    joy_core::git_ops::auto_git_post_command(
        &ctx.root,
        &format!("status {} {old_status} -> {new_status}", item.id),
        &log_user,
    );

    Ok(())
}
