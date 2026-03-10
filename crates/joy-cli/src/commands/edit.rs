// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::Utc;
use clap::Args;

use joy_core::items;
use joy_core::model::item::Priority;
use joy_core::store;

#[derive(Args)]
pub struct EditArgs {
    /// Item ID (e.g. IT-0001)
    id: String,

    /// New title
    #[arg(short, long)]
    title: Option<String>,

    /// New priority: low, medium, high, critical
    #[arg(short, long)]
    priority: Option<String>,

    /// Set parent item ID (use "none" to remove)
    #[arg(long, alias = "epic")]
    parent: Option<String>,

    /// New description
    #[arg(short, long)]
    description: Option<String>,

    /// Set milestone (use "none" to remove)
    #[arg(short, long)]
    milestone: Option<String>,

    /// Tags (comma-separated, replaces existing)
    #[arg(long)]
    tags: Option<String>,

    /// Dependencies (comma-separated IDs, replaces existing)
    #[arg(long)]
    deps: Option<String>,

    /// Set assignee email (use "none" to remove)
    #[arg(short, long)]
    assignee: Option<String>,
}

pub fn run(args: EditArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let mut item = items::load_item(&root, &args.id)?;
    let mut changed = false;

    if let Some(title) = args.title {
        item.title = title;
        changed = true;
    }

    if let Some(ref p) = args.priority {
        item.priority = p
            .parse::<Priority>()
            .map_err(|e: String| anyhow::anyhow!("{}", e))?;
        changed = true;
    }

    if let Some(ref parent) = args.parent {
        if parent == "none" {
            item.parent = None;
        } else {
            if let Ok(parent_item) = items::load_item(&root, parent) {
                if !parent_item.is_active() {
                    eprintln!("Warning: parent {} is {}.", parent, parent_item.status);
                }
            }
            item.parent = Some(parent.clone());
        };
        changed = true;
    }

    if let Some(desc) = args.description {
        item.description = Some(desc);
        changed = true;
    }

    if let Some(ref ms) = args.milestone {
        item.milestone = if ms == "none" { None } else { Some(ms.clone()) };
        changed = true;
    }

    if let Some(ref tags) = args.tags {
        item.tags = tags.split(',').map(|s| s.trim().to_string()).collect();
        changed = true;
    }

    if let Some(ref deps) = args.deps {
        item.deps = if deps.is_empty() {
            Vec::new()
        } else {
            deps.split(',').map(|s| s.trim().to_string()).collect()
        };
        changed = true;
    }

    if let Some(ref assignee) = args.assignee {
        item.assignee = if assignee == "none" {
            None
        } else {
            Some(assignee.clone())
        };
        changed = true;
    }

    if !changed {
        println!("Nothing to change. Use flags like --title, --priority, --parent, etc.");
        return Ok(());
    }

    item.updated = Utc::now();
    items::update_item(&root, &item)?;

    println!("Updated {} {}", item.id, item.title);

    Ok(())
}
