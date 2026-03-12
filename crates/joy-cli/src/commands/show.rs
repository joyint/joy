// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::{DateTime, Local};
use clap::Args;

use joy_core::items;
use joy_core::store;

use crate::color;

#[derive(Args)]
pub struct ShowArgs {
    /// Item ID (e.g. IT-0001)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
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
        "{} {}{}",
        color::label("Type:    "),
        color::item_type_indicator(&item.item_type),
        color::item_type(&item.item_type)
    );
    println!(
        "{} {}{}",
        color::label("Status:  "),
        color::status_indicator(&item.status),
        color::status(&item.status)
    );
    println!(
        "{} {}",
        color::label("Priority:"),
        color::priority(&item.priority)
    );

    if let Some(ref parent) = item.parent {
        println!("{} {}", color::label("Parent:  "), color::id(parent));
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
    if let Some(ref version) = item.version {
        println!("{} {}", color::label("Version: "), version);
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

    if item.is_blocked_by(&all_items) {
        let blockers: Vec<_> = all_items
            .iter()
            .filter(|i| item.deps.contains(&i.id) && i.is_active())
            .collect();
        println!("\n  {}", color::blocked("BLOCKED"));
        for blocker in &blockers {
            println!(
                "    {} {} [{}]",
                color::id(&blocker.id),
                blocker.title,
                color::status(&blocker.status)
            );
        }
    }

    if let Some(ref desc) = item.description {
        println!("\n{}", desc.trim_end());
    }

    if !item.comments.is_empty() {
        println!("\n{}:", color::heading("Comments"));
        for (i, comment) in item.comments.iter().enumerate() {
            if i > 0 {
                println!();
            }
            let local_dt: DateTime<Local> = comment.date.with_timezone(&Local);
            let date_str = local_dt.format("%Y-%m-%d %H:%M").to_string();
            println!(
                "{} [{}]",
                color::label(&date_str),
                color::user(&comment.author),
            );
            for line in comment.text.lines() {
                println!("  {line}");
            }
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
