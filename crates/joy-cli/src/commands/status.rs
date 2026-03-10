// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::Utc;
use clap::Args;

use joy_core::items;
use joy_core::model::item::Status;
use joy_core::store;

use crate::color;

#[derive(Args)]
pub struct StatusArgs {
    /// Item ID (e.g. IT-0001)
    id: String,

    /// New status: new, open, in-progress, review, closed, deferred
    status: String,
}

impl StatusArgs {
    pub fn new(id: String, status: String) -> Self {
        Self { id, status }
    }
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

    // Warn when closing an item that has open children
    if matches!(new_status, Status::Closed) {
        let all_items = items::load_items(&root)?;
        let open_children: Vec<_> = all_items
            .iter()
            .filter(|i| i.parent.as_deref() == Some(&item.id) && i.is_active())
            .collect();
        if !open_children.is_empty() {
            eprintln!(
                "Warning: {} has {} open child item(s):",
                color::id(&item.id),
                open_children.len()
            );
            for child in &open_children {
                eprintln!(
                    "  {} {} [{}]",
                    color::id(&child.id),
                    child.title,
                    color::status(&child.status)
                );
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
                color::id(&item.id),
                open_deps.len()
            );
            for dep in &open_deps {
                eprintln!(
                    "  {} {} [{}]",
                    color::id(&dep.id),
                    dep.title,
                    color::status(&dep.status)
                );
            }
        }
    }

    item.status = new_status.clone();
    item.updated = Utc::now();
    items::update_item(&root, &item)?;

    println!(
        "{} {} -> {}",
        color::id(&item.id),
        color::status(&old_status),
        color::status(&new_status)
    );

    // Auto-close parent when all children are closed
    if let (Status::Closed, Some(ref parent_id)) = (&new_status, &item.parent) {
        let all_items = items::load_items(&root)?;
        let has_open_siblings = all_items
            .iter()
            .any(|i| i.parent.as_deref() == Some(parent_id) && i.is_active());

        if !has_open_siblings {
            if let Ok(mut parent) = items::load_item(&root, parent_id) {
                if parent.is_active() {
                    let parent_old = parent.status.clone();
                    parent.status = Status::Closed;
                    parent.updated = Utc::now();
                    items::update_item(&root, &parent)?;
                    println!(
                        "{} {} -> {} (all children closed)",
                        color::id(&parent.id),
                        color::status(&parent_old),
                        color::status(&parent.status)
                    );
                }
            }
        }
    }

    Ok(())
}
