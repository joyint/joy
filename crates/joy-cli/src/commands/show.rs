// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use clap::Args;

use joy_core::items;
use joy_core::store;

#[derive(Args)]
pub struct ShowArgs {
    /// Item ID (e.g. IT-0001)
    id: String,
}

pub fn run(args: ShowArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let item = items::load_item(&root, &args.id)?;
    let all_items = items::load_items(&root)?;

    println!("{} {}", item.id, item.title);
    println!("{}", "-".repeat(60));
    println!("Type:     {}", item.item_type);
    println!("Status:   {}", item.status);
    println!("Priority: {}", item.priority);

    if let Some(ref epic) = item.epic {
        println!("Epic:     {}", epic);
    }
    if let Some(ref assignee) = item.assignee {
        println!("Assignee: {}", assignee);
    }
    if let Some(ref milestone) = item.milestone {
        println!("Milestone: {}", milestone);
    }
    if !item.tags.is_empty() {
        println!("Tags:     {}", item.tags.join(", "));
    }

    if !item.deps.is_empty() {
        println!("\nDependencies:");
        for dep_id in &item.deps {
            let status_info = all_items
                .iter()
                .find(|i| &i.id == dep_id)
                .map(|i| format!("{} [{}]", i.title, i.status))
                .unwrap_or_else(|| "(not found)".to_string());
            println!("  {} {}", dep_id, status_info);
        }
    }

    let blocked = item.is_blocked_by(&all_items);
    if blocked {
        println!("\n  ** BLOCKED by open dependencies **");
    }

    if let Some(ref desc) = item.description {
        println!("\n{}", desc.trim_end());
    }

    if !item.comments.is_empty() {
        println!("\nComments:");
        for comment in &item.comments {
            println!(
                "  {} ({}): {}",
                comment.author,
                comment.date.format("%Y-%m-%d %H:%M"),
                comment.text
            );
        }
    }

    println!("\nCreated: {}", item.created.format("%Y-%m-%d %H:%M"));
    println!("Updated: {}", item.updated.format("%Y-%m-%d %H:%M"));

    Ok(())
}
