// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use clap::Args;

use joy_core::items;
use joy_core::model::item::{Item, ItemType, Status};
use joy_core::store;
use joy_core::vcs::{default_vcs, Vcs};

use crate::color;

#[derive(Args)]
pub struct ReleaseArgs {
    /// Version tag (e.g. v0.5.0). Omit to detect from git tags.
    version: Option<String>,
}

pub fn run(args: ReleaseArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let vcs = default_vcs();

    let version = match args.version {
        Some(v) => v,
        None => match vcs.latest_version_tag(&root) {
            Ok(Some(tag)) => tag,
            Ok(None) => {
                println!("No version tags found. Usage: joy release <VERSION>");
                return Ok(());
            }
            Err(_) => {
                println!("No version tags found. Usage: joy release <VERSION>");
                return Ok(());
            }
        },
    };

    let all_items = items::load_items(&root)?;
    let matched: Vec<&Item> = all_items
        .iter()
        .filter(|i| i.version.as_deref() == Some(version.as_str()))
        .collect();

    if matched.is_empty() {
        println!("No items tagged with {version}.");
        return Ok(());
    }

    println!("{}", color::heading(&format!("Release {version}")));
    println!("{}", color::label(&"-".repeat(60)));

    let type_order = [
        ItemType::Epic,
        ItemType::Story,
        ItemType::Task,
        ItemType::Bug,
        ItemType::Rework,
        ItemType::Decision,
        ItemType::Idea,
    ];

    for item_type in &type_order {
        let group: Vec<&&Item> = matched
            .iter()
            .filter(|i| &i.item_type == item_type)
            .collect();

        if group.is_empty() {
            continue;
        }

        println!(
            "\n{} {}:",
            color::item_type_indicator(item_type),
            color::heading(&format!("{item_type}"))
        );

        for item in &group {
            println!(
                "  {} {} [{}]",
                color::id(&item.id),
                item.title,
                color::status(&item.status)
            );
        }
    }

    let closed_count = matched
        .iter()
        .filter(|i| i.status == Status::Closed)
        .count();
    let open_count = matched.len() - closed_count;

    println!();
    println!(
        "{} items: {} closed, {} open",
        matched.len(),
        closed_count,
        open_count
    );

    Ok(())
}
