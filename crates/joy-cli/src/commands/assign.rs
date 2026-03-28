// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::{bail, Result};
use chrono::Utc;
use clap::Args;

use joy_core::identity;
use joy_core::items;
use joy_core::model::item::{Assignee, Capability};
use joy_core::store;

use crate::color;

#[derive(Args)]
pub struct AssignArgs {
    /// Item ID (e.g. IT-0001)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    id: String,

    /// Member ID (email or ai:tool@joy). Omit to use git config user.email.
    member: Option<String>,

    /// Capabilities to assign (comma-separated, e.g. implement,review)
    #[arg(long = "as")]
    capabilities: Option<String>,

    /// Remove a member's assignment
    #[arg(long)]
    unassign: bool,

    /// Override identity (email or ai:tool@joy).
    #[arg(long)]
    author: Option<String>,
}

pub fn run(args: AssignArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let mut item = items::load_item(&root, &args.id)?;

    let member = match args.member {
        Some(m) => m,
        None => {
            let id = identity::resolve_identity_with(&root, args.author.as_deref())
                .map_err(|e| anyhow::anyhow!("{e}. Provide member ID explicitly."))?;
            crate::warn_ai_members(&root, &id);
            id.member
        }
    };

    joy_core::guard::enforce(
        &root,
        &joy_core::guard::Action::AssignItem,
        &item.id,
        args.author.as_deref(),
    )?;

    // Validate format
    if !member.contains('@') && !member.starts_with("ai:") {
        bail!("invalid member format: expected email or ai:tool@joy");
    }

    if args.unassign {
        let before = item.assignees.len();
        item.assignees.retain(|a| a.member != member);
        if item.assignees.len() == before {
            println!("{} is not assigned to {}.", color::id(&item.id), member);
            return Ok(());
        }
        item.updated = Utc::now();
        items::update_item(&root, &item)?;
        let id = identity::resolve_identity_with(&root, args.author.as_deref())
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        joy_core::event_log::log_event_as(
            &root,
            joy_core::event_log::EventType::ItemUnassigned,
            &item.id,
            Some(&member),
            &id.log_user(),
        );
        println!("Unassigned {} from {}", member, color::id(&item.id));
        joy_core::git_ops::auto_git_post_command(
            &root,
            &format!("unassign {} {}", item.id, member),
            &id.log_user(),
        );
        return Ok(());
    }

    let caps: Vec<Capability> = match args.capabilities {
        Some(ref s) => s
            .split(',')
            .map(|c| {
                c.trim()
                    .parse::<Capability>()
                    .map_err(|e| anyhow::anyhow!("{}", e))
            })
            .collect::<Result<Vec<_>>>()?,
        None => Vec::new(),
    };

    // Update existing assignment or add new one
    if let Some(existing) = item.assignees.iter_mut().find(|a| a.member == member) {
        existing.capabilities = caps.clone();
    } else {
        item.assignees.push(Assignee {
            member: member.clone(),
            capabilities: caps.clone(),
        });
    }

    item.updated = Utc::now();
    items::update_item(&root, &item)?;

    let id = identity::resolve_identity_with(&root, args.author.as_deref())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    joy_core::event_log::log_event_as(
        &root,
        joy_core::event_log::EventType::ItemAssigned,
        &item.id,
        Some(&member),
        &id.log_user(),
    );

    if caps.is_empty() {
        println!("Assigned {} to {}", color::id(&item.id), member);
    } else {
        let cap_names: Vec<String> = caps.iter().map(|c| c.to_string()).collect();
        println!(
            "Assigned {} to {} as {}",
            color::id(&item.id),
            member,
            cap_names.join(", ")
        );
    }

    joy_core::git_ops::auto_git_post_command(
        &root,
        &format!("assign {} {}", item.id, member),
        &id.log_user(),
    );

    Ok(())
}
