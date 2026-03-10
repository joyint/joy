// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use clap::Args;

use joy_core::items;
use joy_core::store;

use crate::color;

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

    println!("{} {}", color::id(&item.id), color::heading(&item.title));
    println!("{}", color::label(&"-".repeat(60)));
    println!(
        "{} {}",
        color::label("Type:    "),
        color::item_type(&item.item_type)
    );
    println!(
        "{} {}",
        color::label("Status:  "),
        color::status(&item.status)
    );
    println!(
        "{} {}",
        color::label("Priority:"),
        color::priority(&item.priority)
    );

    if let Some(ref epic) = item.epic {
        println!("{} {}", color::label("Epic:    "), color::id(epic));
    }
    if let Some(ref assignee) = item.assignee {
        println!("{} {}", color::label("Assignee:"), assignee);
    }
    if let Some(ref milestone) = item.milestone {
        println!("{} {}", color::label("Milestone:"), color::id(milestone));
    }
    if !item.tags.is_empty() {
        println!("{} {}", color::label("Tags:    "), item.tags.join(", "));
    }

    if !item.deps.is_empty() {
        println!("\n{}:", color::heading("Dependencies"));
        for dep_id in &item.deps {
            let dep_info = all_items
                .iter()
                .find(|i| &i.id == dep_id)
                .map(|i| format!("{} [{}]", i.title, color::status(&i.status)))
                .unwrap_or_else(|| "(not found)".to_string());
            println!("  {} {}", color::id(dep_id), dep_info);
        }
    }

    let blocked = item.is_blocked_by(&all_items);
    if blocked {
        println!(
            "\n  {}",
            color::blocked("** BLOCKED by open dependencies **")
        );
    }

    if let Some(ref desc) = item.description {
        println!("\n{}", desc.trim_end());
    }

    if !item.comments.is_empty() {
        println!("\n{}:", color::heading("Comments"));
        for comment in &item.comments {
            println!(
                "  {} {}: {}",
                color::heading(&comment.author),
                color::label(&comment.date.format("(%Y-%m-%d %H:%M)").to_string()),
                comment.text
            );
        }
    }

    println!(
        "\n{} {}",
        color::label("Created:"),
        color::label(&item.created.format("%Y-%m-%d %H:%M").to_string())
    );
    println!(
        "{} {}",
        color::label("Updated:"),
        color::label(&item.updated.format("%Y-%m-%d %H:%M").to_string())
    );

    Ok(())
}
