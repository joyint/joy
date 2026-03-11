// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use clap::Args;

use joy_core::items;
use joy_core::store;

use crate::color;

#[derive(Args)]
pub struct FindArgs {
    /// Search text (case-insensitive, matches title and description)
    query: String,
}

pub fn run(args: FindArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let all_items = items::load_items(&root)?;
    let query = args.query.to_lowercase();

    let matches: Vec<_> = all_items
        .iter()
        .filter(|item| {
            item.title.to_lowercase().contains(&query)
                || item
                    .description
                    .as_deref()
                    .is_some_and(|d| d.to_lowercase().contains(&query))
        })
        .collect();

    if matches.is_empty() {
        println!("No items matching \"{}\".", args.query);
        return Ok(());
    }

    for item in &matches {
        println!(
            "  {} {} [{}] [{}]",
            color::id(&item.id),
            item.title,
            color::item_type(&item.item_type),
            color::status(&item.status),
        );
    }

    println!(
        "\n{}",
        color::label(&format!("{} match(es)", matches.len()))
    );

    Ok(())
}
