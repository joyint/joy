// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::{bail, Result};
use clap::Args;

use joy_core::items;
use joy_core::model::item::{Item, ItemType, Priority, Status};
use joy_core::store;

#[derive(Args)]
#[command(
    override_usage = "joy add [TYPE] [TITLE] [OPTIONS]",
    after_help = "\
Item IDs use the project acronym as prefix and are auto-generated:
  ACRONYM-0001 to ACRONYM-FFFF (e.g. JOY-0001, JOY-00AF)

Use --id to assign a specific ID manually."
)]
pub struct AddArgs {
    /// Item type: epic, story, task, bug, rework, decision, idea
    #[arg(index = 1, value_name = "TYPE")]
    pos_type: Option<String>,

    /// Item title
    #[arg(index = 2, value_name = "TITLE")]
    pos_title: Option<String>,

    /// Item title (alternative to positional)
    #[arg(short, long, hide = true)]
    title: Option<String>,

    /// Item type (alternative to positional): epic, story, task, bug, rework, decision, idea
    #[arg(short = 'T', long = "type", hide = true)]
    item_type: Option<String>,

    /// Priority: low, medium, high, critical
    #[arg(short, long, default_value = "medium")]
    priority: String,

    /// Parent item ID (epic, story, or task)
    #[arg(long)]
    parent: Option<String>,

    /// Size (1-7): 1=trivial, 2=small, 3=medium, 4=large, 5=major, 6=heavy, 7=massive
    #[arg(short, long)]
    size: Option<u8>,

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

    /// Version tag (e.g. v0.5.0)
    #[arg(short = 'v', long)]
    version: Option<String>,
}

pub fn run(args: AddArgs) -> Result<()> {
    // Show help when called without any arguments
    if args.pos_type.is_none()
        && args.pos_title.is_none()
        && args.title.is_none()
        && args.item_type.is_none()
    {
        use clap::CommandFactory;
        // Build a throwaway Cli just to extract the add subcommand help
        let mut cmd = crate::Cli::command();
        let sub = cmd.find_subcommand_mut("add").unwrap();
        sub.print_help()?;
        std::process::exit(0);
    }

    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let type_str = args
        .item_type
        .or(args.pos_type)
        .ok_or_else(|| anyhow::anyhow!("type is required: joy add <TYPE> <TITLE> or --type"))?;

    let title = args
        .title
        .or(args.pos_title)
        .ok_or_else(|| anyhow::anyhow!("title is required: joy add <TYPE> \"<TITLE>\""))?;

    let item_type: ItemType = type_str
        .parse()
        .map_err(|e: String| anyhow::anyhow!("{}", e))?;

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
        None => {
            let acronym = joy_core::store::load_acronym(&root)?;
            items::next_id(&root, &acronym)?
        }
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

    item.version = args.version;
    if let Some(s) = args.size {
        if !(1..=7).contains(&s) {
            bail!("size must be between 1 and 7");
        }
        item.size = Some(s);
    }

    if let Some(ref s) = args.status {
        item.status = s
            .parse::<Status>()
            .map_err(|e: String| anyhow::anyhow!("{}", e))?;
    }

    // Validate parent exists as an item
    if let Some(ref parent_id) = item.parent {
        match items::load_item(&root, parent_id) {
            Ok(parent) => {
                if !parent.is_active() {
                    eprintln!("Warning: parent {} is {}.", parent_id, parent.status);
                }
            }
            Err(_) => {
                if parent_id.contains("-MS-") {
                    bail!(
                        "{} is a milestone, not an item. Use `joy milestone link <ID> {}` instead.",
                        parent_id,
                        parent_id
                    );
                }
                bail!("parent {} is not a valid item ID.", parent_id);
            }
        }
    }

    items::save_item(&root, &item)?;

    joy_core::event_log::log_event(
        &root,
        joy_core::event_log::EventType::ItemCreated,
        &id,
        Some(&title),
    );

    println!("Created {} {}", id, title);

    Ok(())
}
