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
}

pub fn run(args: AssignArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    joy_core::capabilities::warn_unless_capable(&root, Capability::Assign);

    let mut item = items::load_item(&root, &args.id)?;

    let member = match args.member {
        Some(m) => m,
        None => {
            let id = identity::resolve_identity(&root)
                .map_err(|e| anyhow::anyhow!("{e}. Provide member ID explicitly."))?;
            crate::warn_ai_members(&root, &id);
            id.member
        }
    };

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
        joy_core::event_log::log_event(
            &root,
            joy_core::event_log::EventType::ItemUnassigned,
            &item.id,
            Some(&member),
        );
        println!("Unassigned {} from {}", member, color::id(&item.id));
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

    joy_core::event_log::log_event(
        &root,
        joy_core::event_log::EventType::ItemAssigned,
        &item.id,
        Some(&member),
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

    Ok(())
}
