// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::NaiveDate;
use clap::{Args, Subcommand};

use joy_core::items;
use joy_core::milestones;
use joy_core::model::milestone::Milestone;
use joy_core::store;

use super::ls::effective_milestone;
use crate::color;

#[derive(Args)]
pub struct MilestoneArgs {
    #[command(subcommand)]
    command: MilestoneCommand,
}

#[derive(Subcommand)]
enum MilestoneCommand {
    /// Create a new milestone
    Add(AddArgs),
    /// List milestones
    Ls,
    /// Show milestone details with progress
    Show(ShowArgs),
    /// Delete a milestone
    Rm(RmArgs),
    /// Link an item to a milestone
    Link(LinkArgs),
    /// Modify a milestone
    Edit(EditArgs),
    /// Unlink an item from its milestone
    Unlink(UnlinkArgs),
}

#[derive(Args)]
struct AddArgs {
    /// Milestone title
    title: String,

    /// Target date (YYYY-MM-DD)
    #[arg(long)]
    date: Option<String>,

    /// Description
    #[arg(short, long)]
    description: Option<String>,
}

#[derive(Args)]
struct EditArgs {
    /// Milestone ID (e.g. MS-01)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    id: String,

    /// New title
    #[arg(short, long)]
    title: Option<String>,

    /// New target date (YYYY-MM-DD, use "none" to remove)
    #[arg(long)]
    date: Option<String>,

    /// New description (use "none" to remove)
    #[arg(short, long)]
    description: Option<String>,
}

#[derive(Args)]
struct ShowArgs {
    /// Milestone ID (e.g. MS-01)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    id: String,
}

#[derive(Args)]
struct RmArgs {
    /// Milestone ID (e.g. MS-01)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    id: String,

    /// Skip confirmation
    #[arg(long)]
    force: bool,
}

#[derive(Args)]
struct LinkArgs {
    /// Item ID to link
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    item_id: String,

    /// Milestone ID to link to
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    ms_id: String,
}

#[derive(Args)]
struct UnlinkArgs {
    /// Item ID to unlink from its milestone
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    item_id: String,
}

pub fn run(args: MilestoneArgs) -> Result<()> {
    match args.command {
        MilestoneCommand::Add(a) => run_add(a),
        MilestoneCommand::Ls => run_ls(),
        MilestoneCommand::Show(a) => run_show(a),
        MilestoneCommand::Rm(a) => run_rm(a),
        MilestoneCommand::Edit(a) => run_edit(a),
        MilestoneCommand::Link(a) => run_link(a),
        MilestoneCommand::Unlink(a) => run_unlink(a),
    }
}

fn run_add(args: AddArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let acronym = store::load_acronym(&root)?;
    let id = milestones::next_id(&root, &acronym)?;
    let mut ms = Milestone::new(id.clone(), args.title);

    if let Some(ref date_str) = args.date {
        ms.date = Some(
            NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                .map_err(|_| anyhow::anyhow!("invalid date format, use YYYY-MM-DD"))?,
        );
    }

    ms.description = args.description;

    milestones::save_milestone(&root, &ms)?;

    joy_core::event_log::log_event(
        &root,
        joy_core::event_log::EventType::MilestoneCreated,
        &id,
        Some(&ms.title),
    );

    println!("Created {} {}", color::id(&id), ms.title);
    if let Some(date) = ms.date {
        println!("  Date: {date}");
    }

    Ok(())
}

fn run_ls() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let milestones = milestones::load_milestones(&root)?;
    let all_items = items::load_items(&root)?;

    if milestones.is_empty() {
        println!("No milestones.");
        return Ok(());
    }

    for ms in &milestones {
        let linked: Vec<_> = all_items
            .iter()
            .filter(|i| effective_milestone(i, &all_items) == Some(&ms.id))
            .collect();
        let closed = linked.iter().filter(|i| !i.is_active()).count();
        let total = linked.len();

        let date_str = ms.date.map(|d| format!(" ({d})")).unwrap_or_default();

        let progress = if total > 0 {
            format!(" [{closed}/{total}]")
        } else {
            String::new()
        };

        println!(
            "  {} {}{}{}",
            color::id(&ms.id),
            ms.title,
            color::label(&date_str),
            progress
        );
    }

    Ok(())
}

fn run_show(args: ShowArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let ms = milestones::load_milestone(&root, &args.id)?;
    let all_items = items::load_items(&root)?;
    let linked: Vec<_> = all_items
        .iter()
        .filter(|i| effective_milestone(i, &all_items) == Some(&ms.id))
        .collect();

    println!("{} {}", color::id(&ms.id), color::heading(&ms.title));

    if let Some(date) = ms.date {
        println!("{} {date}", color::label("Date:"));
    }

    if let Some(ref desc) = ms.description {
        println!("\n{}", desc.trim_end());
    }

    let closed = linked.iter().filter(|i| !i.is_active()).count();
    let total = linked.len();
    let blocked: Vec<_> = linked
        .iter()
        .filter(|i| i.is_blocked_by(&all_items))
        .collect();

    println!(
        "\n{} {closed}/{total} items closed",
        color::label("Progress:")
    );

    if !blocked.is_empty() {
        println!(
            "\n{} ({} blocked item(s)):",
            color::blocked("Risks"),
            blocked.len()
        );
        for item in &blocked {
            println!(
                "  {} {} [{}]",
                color::id(&item.id),
                item.title,
                color::status(&item.status)
            );
            for dep_id in &item.deps {
                if let Some(dep) = all_items.iter().find(|i| i.id == *dep_id) {
                    if dep.is_active() {
                        println!(
                            "    {} {} {} [{}]",
                            color::label("blocked by"),
                            color::id(&dep.id),
                            dep.title,
                            color::status(&dep.status)
                        );
                    }
                }
            }
        }
    }

    if !linked.is_empty() {
        println!("\n{}:", color::heading("Items"));
        for item in &linked {
            let blocked_marker = if item.is_blocked_by(&all_items) {
                format!(" {}", color::blocked("[blocked]"))
            } else {
                String::new()
            };
            println!(
                "  {} {} [{}]{}",
                color::id(&item.id),
                item.title,
                color::status(&item.status),
                blocked_marker
            );
        }
    }

    Ok(())
}

fn run_rm(args: RmArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let ms = milestones::load_milestone(&root, &args.id)?;

    if !args.force {
        eprint!(
            "Delete milestone {} {}? (y/N): ",
            color::id(&ms.id),
            ms.title
        );
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    // Remove milestone references from items
    let all_items = items::load_items(&root)?;
    for mut item in all_items {
        if item.milestone.as_deref() == Some(&ms.id) {
            item.milestone = None;
            item.updated = chrono::Utc::now();
            items::update_item(&root, &item)?;
            println!("  Unlinked {}", color::id(&item.id));
        }
    }

    milestones::delete_milestone(&root, &args.id)?;

    joy_core::event_log::log_event(
        &root,
        joy_core::event_log::EventType::MilestoneDeleted,
        &ms.id,
        Some(&ms.title),
    );

    println!("Deleted {} {}", color::id(&ms.id), ms.title);

    Ok(())
}

fn run_edit(args: EditArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let mut ms = milestones::load_milestone(&root, &args.id)?;
    let mut changed = false;

    if let Some(title) = args.title {
        ms.title = title;
        changed = true;
    }

    if let Some(ref date_str) = args.date {
        if date_str == "none" {
            ms.date = None;
        } else {
            ms.date = Some(
                NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                    .map_err(|_| anyhow::anyhow!("invalid date format, use YYYY-MM-DD"))?,
            );
        }
        changed = true;
    }

    if let Some(ref desc) = args.description {
        ms.description = if desc == "none" {
            None
        } else {
            Some(desc.clone())
        };
        changed = true;
    }

    if !changed {
        println!("Nothing to change. Use flags like --title, --date, --description.");
        return Ok(());
    }

    milestones::update_milestone(&root, &ms)?;

    joy_core::event_log::log_event(
        &root,
        joy_core::event_log::EventType::MilestoneUpdated,
        &ms.id,
        Some(&ms.title),
    );

    println!("Updated {} {}", color::id(&ms.id), ms.title);

    Ok(())
}

fn run_link(args: LinkArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    // Verify milestone exists
    let ms = milestones::load_milestone(&root, &args.ms_id)?;

    let mut item = items::load_item(&root, &args.item_id)?;
    item.milestone = Some(args.ms_id.clone());
    item.updated = chrono::Utc::now();
    items::update_item(&root, &item)?;

    joy_core::event_log::log_event(
        &root,
        joy_core::event_log::EventType::MilestoneLinked,
        &item.id,
        Some(&ms.id),
    );

    println!(
        "Linked {} to {} {}",
        color::id(&item.id),
        color::id(&ms.id),
        ms.title
    );

    Ok(())
}

fn run_unlink(args: UnlinkArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let mut item = items::load_item(&root, &args.item_id)?;

    match &item.milestone {
        Some(ms_id) => {
            let old_ms_id = ms_id.clone();
            item.milestone = None;
            item.updated = chrono::Utc::now();
            items::update_item(&root, &item)?;

            joy_core::event_log::log_event(
                &root,
                joy_core::event_log::EventType::MilestoneUnlinked,
                &item.id,
                Some(&old_ms_id),
            );

            println!(
                "Unlinked {} from {}",
                color::id(&item.id),
                color::id(&old_ms_id)
            );
        }
        None => {
            println!("{} is not linked to any milestone.", color::id(&item.id));
        }
    }

    Ok(())
}
