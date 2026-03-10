// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;

use joy_core::items;
use joy_core::model::item::{Item, Status};
use joy_core::store;

pub fn run() -> Result<()> {
    let cwd = std::env::current_dir()?;

    let root = match store::find_project_root(&cwd) {
        Some(r) => r,
        None => {
            println!("joy v0.1.0 -- run `joy init` to get started");
            return Ok(());
        }
    };

    let all_items = items::load_items(&root)?;

    if all_items.is_empty() {
        println!("No items. Run `joy add` to create one.");
        return Ok(());
    }

    let groups: &[(Status, &str)] = &[
        (Status::New, "NEW"),
        (Status::Open, "OPEN"),
        (Status::InProgress, "IN PROGRESS"),
        (Status::Review, "REVIEW"),
        (Status::Closed, "CLOSED"),
        (Status::Deferred, "DEFERRED"),
    ];

    let mut total = 0;
    let mut closed = 0;

    for (status, label) in groups {
        let items_in_status: Vec<&Item> =
            all_items.iter().filter(|i| &i.status == status).collect();

        if items_in_status.is_empty() {
            continue;
        }

        let count = items_in_status.len();
        total += count;
        if matches!(status, Status::Closed) {
            closed = count;
        }

        println!("--- {} ({}) ---", label, count);
        for item in &items_in_status {
            let blocked = if item.is_blocked_by(&all_items) {
                " [blocked]"
            } else {
                ""
            };
            println!(
                "  {} {} [{}]{}",
                item.id, item.title, item.priority, blocked
            );
        }
        println!();
    }

    // Summary line
    if total > 0 {
        println!(
            "{} item(s), {} closed, {} active",
            total,
            closed,
            total - closed
        );
    }

    Ok(())
}
