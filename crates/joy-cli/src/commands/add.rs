// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::{bail, Result};
use clap::Args;

use joy_core::items;
use joy_core::model::item::{Item, ItemType, Priority};
use joy_core::store;

#[derive(Args)]
pub struct AddArgs {
    /// Item title
    #[arg(short, long)]
    title: Option<String>,

    /// Item type: epic, story, task, bug, rework, decision
    #[arg(short = 'T', long = "type")]
    item_type: Option<String>,

    /// Priority: low, medium, high, critical
    #[arg(short, long, default_value = "medium")]
    priority: String,

    /// Parent epic ID
    #[arg(short, long)]
    epic: Option<String>,

    /// Description
    #[arg(short, long)]
    description: Option<String>,

    /// Milestone ID
    #[arg(short, long)]
    milestone: Option<String>,

    /// Tags (comma-separated)
    #[arg(long)]
    tags: Option<String>,
}

pub fn run(args: AddArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let title = match args.title {
        Some(t) => t,
        None => {
            bail!("--title is required (interactive mode not yet implemented)");
        }
    };

    let item_type: ItemType = match args.item_type {
        Some(ref t) => t.parse().map_err(|e: String| anyhow::anyhow!("{}", e))?,
        None => {
            bail!("--type is required (interactive mode not yet implemented)");
        }
    };

    let priority: Priority = args
        .priority
        .parse()
        .map_err(|e: String| anyhow::anyhow!("{}", e))?;

    let id = items::next_id(&root, &item_type)?;
    let mut item = Item::new(id.clone(), title.clone(), item_type, priority);
    item.epic = args.epic;
    item.description = args.description;
    item.milestone = args.milestone;
    item.tags = args
        .tags
        .map(|t| t.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    items::save_item(&root, &item)?;

    println!("Created {} {}", id, title);

    Ok(())
}
