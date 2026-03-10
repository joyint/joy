// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::Utc;
use clap::Args;

use joy_core::items;
use joy_core::store;

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
        println!("Unassigned {} from {}", color::id(&item.id), old);
        return Ok(());
    }

    let email = match args.email {
        Some(e) => e,
        None => get_git_email()?,
    };

    // Basic validation: must contain @ or be an agent: identity
    if !email.contains('@') && !email.starts_with("agent:") {
        anyhow::bail!("invalid assignee format: expected email or agent:role@joy");
    }

    item.assignee = Some(email.clone());
    item.updated = Utc::now();
    items::update_item(&root, &item)?;

    println!(
        "Assigned {} {} to {}",
        color::id(&item.id),
        item.title,
        email
    );

    Ok(())
}

fn get_git_email() -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["config", "user.email"])
        .output()
        .map_err(|_| anyhow::anyhow!("failed to run git config user.email"))?;

    if !output.status.success() {
        anyhow::bail!("git config user.email not set. Provide email explicitly.");
    }

    let email = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if email.is_empty() {
        anyhow::bail!("git config user.email is empty. Provide email explicitly.");
    }

    Ok(email)
}
