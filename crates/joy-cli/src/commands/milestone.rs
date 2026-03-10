// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::NaiveDate;
use clap::{Args, Subcommand};

use joy_core::items;
use joy_core::milestones;
use joy_core::model::milestone::Milestone;
use joy_core::store;

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
struct ShowArgs {
    /// Milestone ID (e.g. MS-01)
    id: String,
}

#[derive(Args)]
struct RmArgs {
    /// Milestone ID (e.g. MS-01)
    id: String,

    /// Skip confirmation
    #[arg(long)]
    force: bool,
}

#[derive(Args)]
struct LinkArgs {
    /// Item ID to link
    item_id: String,

    /// Milestone ID to link to
    ms_id: String,
}

pub fn run(args: MilestoneArgs) -> Result<()> {
    match args.command {
        MilestoneCommand::Add(a) => run_add(a),
        MilestoneCommand::Ls => run_ls(),
        MilestoneCommand::Show(a) => run_show(a),
        MilestoneCommand::Rm(a) => run_rm(a),
        MilestoneCommand::Link(a) => run_link(a),
    }
}

fn run_add(args: AddArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let id = milestones::next_id(&root)?;
    let mut ms = Milestone::new(id.clone(), args.title);

    if let Some(ref date_str) = args.date {
        ms.date = Some(
            NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                .map_err(|_| anyhow::anyhow!("invalid date format, use YYYY-MM-DD"))?,
        );
    }

    ms.description = args.description;

    milestones::save_milestone(&root, &ms)?;
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
            .filter(|i| i.milestone.as_deref() == Some(&ms.id))
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
        .filter(|i| i.milestone.as_deref() == Some(&ms.id))
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
    println!("Deleted {} {}", color::id(&ms.id), ms.title);

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

    println!(
        "Linked {} to {} {}",
        color::id(&item.id),
        color::id(&ms.id),
        ms.title
    );

    Ok(())
}
