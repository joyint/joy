// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::{bail, Result};
use clap::Args;

use joy_core::items;
use joy_core::model::item::{Item, ItemType, Priority, Status};
use joy_core::store;

#[derive(Args)]
pub struct AddArgs {
    /// Item title
    #[arg(short, long)]
    title: Option<String>,

    /// Item type: epic, story, task, bug, rework, decision, idea
    #[arg(short = 'T', long = "type")]
    item_type: Option<String>,

    /// Priority: low, medium, high, critical
    #[arg(short, long, default_value = "medium")]
    priority: String,

    /// Parent item ID (epic, story, or task)
    #[arg(long)]
    parent: Option<String>,

    /// Description
    #[arg(short, long)]
    description: Option<String>,

    /// Milestone ID
    #[arg(short, long)]
    milestone: Option<String>,

    /// Tags (comma-separated)
    #[arg(long)]
    tags: Option<String>,

    /// Explicit item ID (skip auto-generation)
    #[arg(long)]
    id: Option<String>,

    /// Dependencies (comma-separated IDs)
    #[arg(long)]
    deps: Option<String>,

    /// Initial status: new, open, in-progress, review, closed, deferred
    #[arg(short, long)]
    status: Option<String>,
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

    let id = match args.id {
        Some(id) => {
            if items::find_item_file(&root, &id).is_ok() {
                bail!("item {} already exists", id);
            }
            id
        }
        None => items::next_id(&root, &item_type)?,
    };

    let mut item = Item::new(id.clone(), title.clone(), item_type, priority);
    item.parent = args.parent;
    item.description = args.description;
    item.milestone = args.milestone;
    item.tags = args
        .tags
        .map(|t| t.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();
    item.deps = args
        .deps
        .map(|d| d.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    if let Some(ref s) = args.status {
        item.status = s
            .parse::<Status>()
            .map_err(|e: String| anyhow::anyhow!("{}", e))?;
    }

    // Warn if parent is closed
    if let Some(ref parent_id) = item.parent {
        if let Ok(parent) = items::load_item(&root, parent_id) {
            if !parent.is_active() {
                eprintln!("Warning: parent {} is {}.", parent_id, parent.status);
            }
        }
    }

    items::save_item(&root, &item)?;

    println!("Created {} {}", id, title);

    Ok(())
}
