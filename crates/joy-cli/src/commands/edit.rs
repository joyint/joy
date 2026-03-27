// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::Utc;
use clap::Args;

use joy_core::identity;
use joy_core::items;
use joy_core::model::item::{Capability, Priority};
use joy_core::store;

#[derive(Args)]
pub struct EditArgs {
    /// Item ID (e.g. IT-0001)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    id: String,

    /// New title
    #[arg(short, long)]
    title: Option<String>,

    /// New priority: low, medium, high, critical, extreme
    #[arg(short, long)]
    priority: Option<String>,

    /// Set parent item ID (use "none" to remove)
    #[arg(long)]
    parent: Option<String>,

    /// Effort (1-7, use "none" to remove)
    #[arg(short, long)]
    effort: Option<String>,

    /// New description
    #[arg(short, long)]
    description: Option<String>,

    /// Set milestone (use "none" to remove)
    #[arg(short = 'm', long)]
    milestone: Option<String>,

    /// Tags (comma-separated, replaces existing)
    #[arg(long)]
    tags: Option<String>,

    /// Dependencies (comma-separated IDs, replaces existing)
    #[arg(long)]
    deps: Option<String>,

    /// Set assignee (use "none" to clear all). Use `joy assign` for capability-based assignments.
    #[arg(short = 'A', long)]
    assignee: Option<String>,

    /// Set version tag (use "none" to remove)
    #[arg(short = 'v', long)]
    version: Option<String>,

    /// Capabilities (comma-separated, replaces existing)
    #[arg(short = 'c', long)]
    capabilities: Option<String>,
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

    if let Some(ref effort_str) = args.effort {
        if effort_str == "none" {
            item.effort = None;
        } else {
            let e: u8 = effort_str
                .parse()
                .map_err(|_| anyhow::anyhow!("effort must be 1-7 or 'none'"))?;
            if !(1..=7).contains(&e) {
                anyhow::bail!("effort must be between 1 and 7");
            }
            item.effort = Some(e);
        }
        changed = true;
    }

    if let Some(ref parent) = args.parent {
        if parent == "none" {
            item.parent = None;
        } else {
            match items::load_item(&root, parent) {
                Ok(parent_item) => {
                    if !parent_item.is_active() {
                        eprintln!("Warning: parent {} is {}.", parent, parent_item.status);
                    }
                }
                Err(_) => {
                    if parent.contains("-MS-") {
                        anyhow::bail!("{} is a milestone, not an item. Use `joy milestone link {} {}` instead.", parent, item.id, parent);
                    }
                    anyhow::bail!("parent {} is not a valid item ID.", parent);
                }
            }
            item.parent = Some(parent.clone());
        }
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
        item.tags = if tags.is_empty() {
            Vec::new()
        } else {
            tags.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };
        changed = true;
    }

    if let Some(ref deps) = args.deps {
        if deps.is_empty() {
            item.deps = Vec::new();
        } else {
            let new_deps: Vec<String> = deps.split(',').map(|s| s.trim().to_string()).collect();
            for dep_id in &new_deps {
                if let Some(cycle) = items::detect_cycle(&root, &item.id, dep_id)? {
                    anyhow::bail!("circular dependency: {}", cycle.join(" -> "));
                }
            }
            item.deps = new_deps;
        }
        changed = true;
    }

    if let Some(ref version) = args.version {
        item.version = if version == "none" {
            None
        } else {
            Some(version.clone())
        };
        changed = true;
    }

    if let Some(ref caps) = args.capabilities {
        if caps.is_empty() {
            item.capabilities = Vec::new();
        } else {
            item.capabilities = caps
                .split(',')
                .map(|s| {
                    s.trim()
                        .parse::<Capability>()
                        .map_err(|e| anyhow::anyhow!("{}", e))
                })
                .collect::<Result<Vec<_>>>()?;
        }
        changed = true;
    }

    if let Some(ref assignee) = args.assignee {
        joy_core::capabilities::warn_unless_capable(
            &root,
            joy_core::model::item::Capability::Assign,
        );
        if assignee == "none" {
            item.assignees.clear();
        } else {
            // Simple single-assignee via edit: replaces all assignees
            item.assignees = vec![joy_core::model::item::Assignee {
                member: assignee.clone(),
                capabilities: Vec::new(),
            }];
        }
        changed = true;
    }

    if !changed {
        println!("Nothing to change. Use flags like --title, --priority, --parent, etc.");
        return Ok(());
    }

    item.updated = Utc::now();
    items::update_item(&root, &item)?;

    let log_user = identity::resolve_identity(&root)
        .map(|id| id.log_user())
        .unwrap_or_default();
    joy_core::event_log::log_event_as(
        &root,
        joy_core::event_log::EventType::ItemUpdated,
        &item.id,
        Some(&item.title),
        &log_user,
    );

    println!("Updated {} {}", item.id, item.title);

    Ok(())
}
