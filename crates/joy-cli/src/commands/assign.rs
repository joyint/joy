// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::Utc;
use clap::Args;

use joy_core::items;
use joy_core::store;
use joy_core::vcs::Vcs;

use crate::color;

#[derive(Args)]
pub struct AssignArgs {
    /// Item ID (e.g. IT-0001)
    id: String,

    /// Email address or agent identity (e.g. agent:implementer@joy).
    /// Omit to use git config user.email.
    email: Option<String>,

    /// Remove assignment
    #[arg(long)]
    unassign: bool,
}

pub fn run(args: AssignArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let mut item = items::load_item(&root, &args.id)?;

    if args.unassign {
        if item.assignee.is_none() {
            println!("{} is not assigned.", color::id(&item.id));
            return Ok(());
        }
        let old = item.assignee.take().unwrap();
        item.updated = Utc::now();
        items::update_item(&root, &item)?;
        joy_core::event_log::log_event(
            &root,
            joy_core::event_log::EventType::ItemUnassigned,
            &item.id,
            Some(&old),
        );
        println!("Unassigned {} from {}", color::id(&item.id), old);
        return Ok(());
    }

    let email = match args.email {
        Some(e) => e,
        None => joy_core::vcs::default_vcs()
            .user_email()
            .map_err(|e| anyhow::anyhow!("{e}. Provide email explicitly."))?,
    };

    // Basic validation: must contain @ or be an agent: identity
    if !email.contains('@') && !email.starts_with("agent:") {
        anyhow::bail!("invalid assignee format: expected email or agent:role@joy");
    }

    item.assignee = Some(email.clone());
    item.updated = Utc::now();
    items::update_item(&root, &item)?;

    joy_core::event_log::log_event(
        &root,
        joy_core::event_log::EventType::ItemAssigned,
        &item.id,
        Some(&email),
    );

    println!(
        "Assigned {} {} to {}",
        color::id(&item.id),
        item.title,
        email
    );

    Ok(())
}
