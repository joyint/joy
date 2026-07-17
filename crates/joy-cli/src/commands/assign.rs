// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::{bail, Result};
use clap::Args;

use joy_core::guard::Action;
use joy_core::items;
use joy_core::model::item::{Assignee, Capability};

use crate::color;

#[derive(Args)]
pub struct AssignArgs {
    /// Item ID (e.g. IT-0001)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    id: String,

    /// Member ID (email or ai:tool@joy). Omit to use git config user.email.
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_member))]
    member: Option<String>,

    /// Capabilities to assign (comma-separated, e.g. implement,review)
    #[arg(long = "as")]
    capabilities: Option<String>,

    /// Remove a member's assignment
    #[arg(long)]
    unassign: bool,
}

pub fn run(args: AssignArgs) -> Result<()> {
    let ctx = crate::crypt_session::load_context(None)?;

    let mut item = items::load_item(&ctx.root, &args.id)?;

    let member = match args.member {
        Some(m) => m,
        None => ctx.identity.member.id().to_string(),
    };

    ctx.enforce(&Action::AssignItem, &item.id)?;

    // Validate format. In anonymous mode the acting member resolves to an opaque
    // id (e.g. self-assign), so accept that shape too alongside e-mail / ai: ids.
    if !member.contains('@')
        && !member.starts_with("ai:")
        && !joy_core::member_id::is_opaque_member_id(&member)
    {
        bail!("invalid member format: expected email or ai:tool@joy");
    }

    if args.unassign {
        let before = item.assignees.len();
        item.assignees.retain(|a| a.member != member.as_str());
        if item.assignees.len() == before {
            println!(
                "{} is not assigned to {}.",
                color::id(&item.id),
                color::user(&member)
            );
            return Ok(());
        }
        items::touch_for_attribute_change(&mut item, &ctx.log_user());
        items::update_item(&ctx.root, &item)?;
        joy_core::event_log::log_event_as(
            &ctx.root,
            joy_core::event_log::EventType::ItemUnassigned,
            &item.id,
            Some(&member),
            &ctx.log_user(),
        );
        if crate::output::is_json() {
            return crate::output::emit(&item);
        }
        println!(
            "Unassigned {} from {}",
            color::user(&member),
            color::id(&item.id)
        );
        joy_core::git_ops::auto_git_post_command(
            &ctx.root,
            &format!("unassign {} {}", item.id, member),
            &ctx.log_user(),
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
    let before = item.clone();
    if let Some(existing) = item
        .assignees
        .iter_mut()
        .find(|a| a.member == member.as_str())
    {
        existing.capabilities = caps.clone();
    } else {
        item.assignees.push(Assignee {
            member: member.clone().into(),
            capabilities: caps.clone(),
        });
    }

    // Re-assigning an identical assignment is a no-op: no touch, no
    // history entry, no commit.
    if !items::touch_if_changed(&mut item, &before, &ctx.log_user()) {
        if crate::output::is_json() {
            return crate::output::emit(&item);
        }
        println!(
            "{} is already assigned to {}.",
            color::id(&item.id),
            color::user(&member)
        );
        return Ok(());
    }
    items::update_item(&ctx.root, &item)?;

    joy_core::event_log::log_event_as(
        &ctx.root,
        joy_core::event_log::EventType::ItemAssigned,
        &item.id,
        Some(&member),
        &ctx.log_user(),
    );

    if crate::output::is_json() {
        crate::output::emit(&item)?;
    } else if caps.is_empty() {
        println!(
            "Assigned {} to {}",
            color::id(&item.id),
            color::user(&member)
        );
    } else {
        let cap_names: Vec<String> = caps.iter().map(|c| c.to_string()).collect();
        println!(
            "Assigned {} to {} as {}",
            color::id(&item.id),
            color::user(&member),
            cap_names.join(", ")
        );
    }

    joy_core::git_ops::auto_git_post_command(
        &ctx.root,
        &format!("assign {} {}", item.id, member),
        &ctx.log_user(),
    );

    Ok(())
}
