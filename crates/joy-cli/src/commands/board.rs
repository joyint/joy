// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;

use joy_core::items;
use joy_core::model::item::{Item, Status};
use joy_core::store;

use crate::color;

pub fn run() -> Result<()> {
    let cwd = std::env::current_dir()?;

    let root = match store::find_project_root(&cwd) {
        Some(r) => r,
        None => {
            println!("joy v0.2.0 -- run `joy init` to get started");
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

        println!(
            "--- {} ({}) ---",
            color::status_heading(status, label),
            count
        );
        for item in &items_in_status {
            let blocked_str = if item.is_blocked_by(&all_items) {
                format!(" {}", color::blocked("[blocked]"))
            } else {
                String::new()
            };
            println!(
                "  {} {} [{}]{}",
                color::id(&item.id),
                item.title,
                color::priority(&item.priority),
                blocked_str
            );
        }
        println!();
    }

    if total > 0 {
        println!(
            "{}",
            color::label(&format!(
                "{} item(s), {} closed, {} active",
                total,
                closed,
                total - closed
            ))
        );
    }

    Ok(())
}
