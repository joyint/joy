// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use clap::Args;

use joy_core::items;
use joy_core::model::item::{Item, ItemType, Priority, Status};
use joy_core::store;

use crate::color;

#[derive(Args)]
pub struct LsArgs {
    /// Filter by epic ID
    #[arg(long)]
    epic: Option<String>,

    /// Filter by type: epic, story, task, bug, rework, decision
    #[arg(short = 'T', long = "type")]
    item_type: Option<String>,

    /// Filter by status: new, open, in-progress, review, closed, deferred
    #[arg(long)]
    status: Option<String>,

    /// Filter by priority: low, medium, high, critical
    #[arg(long)]
    priority: Option<String>,

    /// Show only items assigned to me (git config user.email)
    #[arg(long)]
    mine: bool,

    /// Show only blocked items
    #[arg(long)]
    blocked: bool,

    /// Show all items (including closed and deferred)
    #[arg(short, long)]
    all: bool,

    /// Show hierarchical tree view (epics with children)
    #[arg(long)]
    tree: bool,
}

pub fn run(args: LsArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let all_items = items::load_items(&root)?;

    if all_items.is_empty() {
        println!("No items. Run `joy add` to create one.");
        return Ok(());
    }

    let my_email = if args.mine {
        Some(get_git_email()?)
    } else {
        None
    };

    let type_filter: Option<ItemType> = args
        .item_type
        .as_deref()
        .map(|t| t.parse().map_err(|e: String| anyhow::anyhow!("{}", e)))
        .transpose()?;

    let status_filter: Option<Status> = args
        .status
        .as_deref()
        .map(|s| s.parse().map_err(|e: String| anyhow::anyhow!("{}", e)))
        .transpose()?;

    let priority_filter: Option<Priority> = args
        .priority
        .as_deref()
        .map(|p| p.parse().map_err(|e: String| anyhow::anyhow!("{}", e)))
        .transpose()?;

    let filtered: Vec<&Item> = all_items
        .iter()
        .filter(|item| {
            // By default, exclude closed and deferred
            if !args.all
                && args.status.is_none()
                && matches!(item.status, Status::Closed | Status::Deferred)
            {
                return false;
            }

            if let Some(ref epic) = args.epic {
                if item.id != *epic && item.epic.as_deref() != Some(epic.as_str()) {
                    return false;
                }
            }

            if let Some(ref t) = type_filter {
                if &item.item_type != t {
                    return false;
                }
            }

            if let Some(ref s) = status_filter {
                if &item.status != s {
                    return false;
                }
            }

            if let Some(ref p) = priority_filter {
                if &item.priority != p {
                    return false;
                }
            }

            if let Some(ref email) = my_email {
                if item.assignee.as_deref() != Some(email.as_str()) {
                    return false;
                }
            }

            if args.blocked && !item.is_blocked_by(&all_items) {
                return false;
            }

            true
        })
        .collect();

    if filtered.is_empty() {
        println!("No matching items.");
        return Ok(());
    }

    if args.tree {
        print_tree(&filtered);
    } else {
        print_table(&filtered, &all_items);
    }

    Ok(())
}

fn print_table(items: &[&Item], all_items: &[Item]) {
    println!(
        "{:<10} {:<12} {:<13} {:<10} TITLE",
        color::heading("ID"),
        color::heading("TYPE"),
        color::heading("STATUS"),
        color::heading("PRIORITY"),
    );
    println!("{}", color::label(&"-".repeat(70)));

    for item in items {
        let blocked_str = if item.is_blocked_by(all_items) {
            format!(" {}", color::blocked("[blocked]"))
        } else {
            String::new()
        };
        // Pad the colored strings to account for ANSI escape sequences
        let id_str = color::id(&item.id);
        let type_str = color::item_type(&item.item_type);
        let status_str = color::status(&item.status);
        let priority_str = color::priority(&item.priority);

        // Use fixed-width formatting on the raw text, then print colored
        println!(
            "{:<10} {:<12} {:<13} {:<10} {}{}",
            pad_colored(&id_str, &item.id, 10),
            pad_colored(&type_str, &item.item_type.to_string(), 12),
            pad_colored(&status_str, &item.status.to_string(), 13),
            pad_colored(&priority_str, &item.priority.to_string(), 10),
            item.title,
            blocked_str
        );
    }

    println!("\n{}", color::label(&format!("{} item(s)", items.len())));
}

fn pad_colored(colored: &str, raw: &str, width: usize) -> String {
    let padding = width.saturating_sub(raw.len());
    format!("{}{}", colored, " ".repeat(padding))
}

fn print_tree(items: &[&Item]) {
    let epics: Vec<&&Item> = items
        .iter()
        .filter(|i| matches!(i.item_type, ItemType::Epic))
        .collect();

    let orphans: Vec<&&Item> = items
        .iter()
        .filter(|i| !matches!(i.item_type, ItemType::Epic) && i.epic.is_none())
        .collect();

    for epic in &epics {
        println!(
            "{} {} [{}]",
            color::id(&epic.id),
            epic.title,
            color::status(&epic.status)
        );
        let children: Vec<&&Item> = items
            .iter()
            .filter(|i| i.epic.as_deref() == Some(&epic.id))
            .collect();
        for (i, child) in children.iter().enumerate() {
            let connector = if i == children.len() - 1 {
                "└──"
            } else {
                "├──"
            };
            println!(
                "  {} {} {} [{}] [{}]",
                color::label(connector),
                color::id(&child.id),
                child.title,
                color::item_type(&child.item_type),
                color::status(&child.status)
            );
        }
    }

    if !orphans.is_empty() {
        if !epics.is_empty() {
            println!();
        }
        for orphan in &orphans {
            println!(
                "{} {} [{}] [{}]",
                color::id(&orphan.id),
                orphan.title,
                color::item_type(&orphan.item_type),
                color::status(&orphan.status)
            );
        }
    }

    println!("\n{}", color::label(&format!("{} item(s)", items.len())));
}

fn get_git_email() -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["config", "user.email"])
        .output()?;
    let email = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if email.is_empty() {
        anyhow::bail!("git config user.email is not set");
    }
    Ok(email)
}
