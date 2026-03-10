// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::Utc;
use clap::Args;

use joy_core::items;
use joy_core::model::item::{ItemType, Status};
use joy_core::store;

#[derive(Args)]
pub struct StatusArgs {
    /// Item ID (e.g. IT-0001)
    id: String,

    /// New status: new, open, in-progress, review, closed, deferred
    status: String,
}

pub fn run(args: StatusArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let new_status: Status = args
        .status
        .parse()
        .map_err(|e: String| anyhow::anyhow!("{}", e))?;

    let mut item = items::load_item(&root, &args.id)?;
    let old_status = item.status.clone();

    // Warn when closing an epic with open children
    if matches!(new_status, Status::Closed) && matches!(item.item_type, ItemType::Epic) {
        let all_items = items::load_items(&root)?;
        let open_children: Vec<_> = all_items
            .iter()
            .filter(|i| i.epic.as_deref() == Some(&item.id) && i.is_active())
            .collect();
        if !open_children.is_empty() {
            eprintln!(
                "Warning: epic {} has {} open child item(s):",
                item.id,
                open_children.len()
            );
            for child in &open_children {
                eprintln!("  {} {} [{}]", child.id, child.title, child.status);
            }
        }
    }

    // Warn when starting an item with open dependencies
    if matches!(new_status, Status::InProgress) {
        let all_items = items::load_items(&root)?;
        let open_deps: Vec<_> = all_items
            .iter()
            .filter(|i| item.deps.contains(&i.id) && i.is_active())
            .collect();
        if !open_deps.is_empty() {
            eprintln!(
                "Warning: {} has {} open dependency(ies):",
                item.id,
                open_deps.len()
            );
            for dep in &open_deps {
                eprintln!("  {} {} [{}]", dep.id, dep.title, dep.status);
            }
        }
    }

    item.status = new_status.clone();
    item.updated = Utc::now();
    items::update_item(&root, &item)?;

    println!("{} {} -> {}", item.id, old_status, new_status);

    // Auto-close epic when all children are closed
    if let (Status::Closed, Some(ref epic_id)) = (&new_status, &item.epic) {
        let all_items = items::load_items(&root)?;
        let has_open_siblings = all_items
            .iter()
            .any(|i| i.epic.as_deref() == Some(epic_id) && i.is_active());

        if !has_open_siblings {
            if let Ok(mut epic) = items::load_item(&root, epic_id) {
                if epic.is_active() {
                    let epic_old = epic.status.clone();
                    epic.status = Status::Closed;
                    epic.updated = Utc::now();
                    items::update_item(&root, &epic)?;
                    println!(
                        "{} {} -> {} (all children closed)",
                        epic.id, epic_old, epic.status
                    );
                }
            }
        }
    }

    Ok(())
}
