// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use clap::Args;

use joy_core::guard::Action;
use joy_core::items;
use joy_core::model::item::{Capability, ItemType, Priority, Validity};

#[derive(Args)]
pub struct EditArgs {
    /// Item ID (e.g. IT-0001)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    id: String,

    /// New title
    #[arg(short, long)]
    title: Option<String>,

    /// Change item type: epic|story|task|bug|rework|decision|idea
    #[arg(short = 'T', long = "type")]
    item_type: Option<String>,

    /// Priority: low|medium|high|critical|extreme
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

    /// Dependencies (CSV; replaces existing)
    #[arg(long)]
    deps: Option<String>,

    /// Set assignee ("none" clears all)
    #[arg(short = 'A', long)]
    assignee: Option<String>,

    /// Set version tag (use "none" to remove)
    #[arg(short = 'v', long)]
    version: Option<String>,

    /// Decision validity: proposed|accepted|rejected|replaced|retired (use "none" to remove)
    #[arg(long)]
    validity: Option<String>,

    /// ID of the item that replaces this one (use "none" to remove); implies validity=replaced
    #[arg(long = "replaced-by")]
    replaced_by: Option<String>,

    /// Capabilities (CSV; replaces existing)
    #[arg(short = 'c', long)]
    capabilities: Option<String>,
}

pub fn run(args: EditArgs) -> Result<()> {
    let ctx = crate::crypt_session::load_context(None)?;

    let mut item = items::load_item(&ctx.root, &args.id)?;
    let mut changed = false;

    if let Some(title) = args.title {
        item.title = title;
        changed = true;
    }

    if let Some(ref t) = args.item_type {
        item.item_type = t
            .parse::<ItemType>()
            .map_err(|e: String| anyhow::anyhow!("{}", e))?;
        changed = true;
    }

    if let Some(ref p) = args.priority {
        item.priority = p
            .parse::<Priority>()
            .map_err(|e: String| anyhow::anyhow!("{}", e))?;
        changed = true;
    }

    if let Some(ref effort_str) = args.effort {
        item.effort = crate::effort::parse_effort(effort_str)?;
        changed = true;
    }

    if let Some(ref parent) = args.parent {
        if parent == "none" {
            item.parent = None;
        } else {
            match items::load_item(&ctx.root, parent) {
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
                if let Some(cycle) = items::detect_cycle(&ctx.root, &item.id, dep_id)? {
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

    if let Some(ref validity) = args.validity {
        item.validity = if validity == "none" {
            None
        } else {
            Some(
                validity
                    .parse::<Validity>()
                    .map_err(|e: String| anyhow::anyhow!("{}", e))?,
            )
        };
        changed = true;
    }

    if let Some(ref replaced) = args.replaced_by {
        if replaced == "none" {
            item.replaced_by = None;
        } else {
            if replaced == &item.id {
                anyhow::bail!("an item cannot be replaced by itself.");
            }
            if items::load_item(&ctx.root, replaced).is_err() {
                anyhow::bail!("replaced_by {} is not a valid item ID.", replaced);
            }
            item.replaced_by = Some(replaced.clone());
            // A successor implies this item is no longer the current one.
            item.validity = Some(Validity::Replaced);
        }
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
        if crate::output::is_json() {
            return crate::output::emit(&item);
        }
        println!("Nothing to change. Use flags like --title, --priority, --parent, etc.");
        return Ok(());
    }

    // Guard check: AssignItem if assignee changed, UpdateItem otherwise
    let action = if args.assignee.is_some() {
        Action::AssignItem
    } else {
        Action::UpdateItem
    };
    ctx.enforce(&action, &item.id)?;

    let log_user = ctx.log_user();
    items::touch_for_attribute_change(&mut item, &log_user);
    items::update_item(&ctx.root, &item)?;
    joy_core::event_log::log_event_as(
        &ctx.root,
        joy_core::event_log::EventType::ItemUpdated,
        &item.id,
        None,
        &log_user,
    );

    if crate::output::is_json() {
        crate::output::emit(&item)?;
    } else {
        println!("Updated {} {}", item.id, item.title);
    }

    joy_core::git_ops::auto_git_post_command(
        &ctx.root,
        &format!("edit {} {}", item.id, item.title),
        &log_user,
    );

    Ok(())
}
