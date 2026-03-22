// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::io::Write;

use anyhow::Result;
use chrono::Utc;

use joy_core::event_log;
use joy_core::items;
use joy_core::model::item::ItemType;
use joy_core::model::release::{Bump, Contributor, Release, ReleaseItem, ReleaseItems};
use joy_core::releases;
use joy_core::store;
use joy_core::vcs::Vcs;

use crate::color;

#[derive(clap::Args)]
pub struct ReleaseArgs {
    #[command(subcommand)]
    command: ReleaseCommand,
}

#[derive(clap::Subcommand)]
enum ReleaseCommand {
    /// Create a new release from closed items since the last release
    Create(CreateArgs),
    /// Show a release or preview the next one
    Show(ShowArgs),
    /// List all releases
    Ls,
}

#[derive(clap::Args)]
struct CreateArgs {
    /// Version bump: patch (default), minor, or major
    bump: Option<String>,

    /// Release title
    #[arg(long)]
    title: Option<String>,

    /// Release description
    #[arg(long)]
    description: Option<String>,

    /// Explicit version (overrides bump argument)
    #[arg(long)]
    version: Option<String>,
}

#[derive(clap::Args)]
struct ShowArgs {
    /// Version to show (omit for next-release preview)
    version: Option<String>,
}

pub fn run(args: ReleaseArgs) -> Result<()> {
    match args.command {
        ReleaseCommand::Create(args) => create(args),
        ReleaseCommand::Show(args) => show(args),
        ReleaseCommand::Ls => ls(),
    }
}

fn create(args: CreateArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;
    let project = store::load_project(&root)?;
    let acronym = project.acronym.as_deref().unwrap_or("JOY");

    // Determine version: Joy releases first, then git tags as fallback
    let previous = releases::latest_version(&root)?.or_else(|| {
        joy_core::vcs::default_vcs()
            .latest_version_tag(&root)
            .ok()
            .flatten()
    });
    let version = if let Some(v) = args.version {
        if v.starts_with('v') {
            v
        } else {
            format!("v{v}")
        }
    } else {
        let bump_str = args.bump.as_deref().unwrap_or("patch");
        let bump: Bump = bump_str
            .parse()
            .map_err(|e: String| anyhow::anyhow!("{}", e))?;
        let current = previous.as_deref().unwrap_or("v0.0.0");
        joy_core::model::release::bump_version(current, bump)
    };

    // Check if release already exists
    if releases::load_release(&root, acronym, &version).is_ok() {
        anyhow::bail!("Release {} already exists", version);
    }

    // Find cutoff from last release event
    let cutoff = event_log::last_release_timestamp(&root)?;

    // Collect closed items since cutoff
    let closed_ids = event_log::closed_item_ids_since(&root, cutoff.as_deref())?;

    if closed_ids.is_empty() {
        println!("No items closed since last release. Nothing to release.");
        return Ok(());
    }

    // Load item data and group by type
    let all_items = items::load_items(&root)?;
    let mut release_items = ReleaseItems::default();

    for id in &closed_ids {
        let item = match all_items.iter().find(|i| &i.id == id) {
            Some(i) => i,
            None => continue,
        };
        let ri = ReleaseItem {
            id: item.id.clone(),
            title: item.title.clone(),
        };
        match item.item_type {
            ItemType::Epic => release_items.epics.push(ri),
            ItemType::Story => release_items.stories.push(ri),
            ItemType::Task => release_items.tasks.push(ri),
            ItemType::Bug => release_items.bugs.push(ri),
            ItemType::Rework => release_items.reworks.push(ri),
            ItemType::Decision => release_items.decisions.push(ri),
            ItemType::Idea => release_items.ideas.push(ri),
        }
    }

    // Build contributor list from event log actors
    let actors = event_log::actors_since(&root, cutoff.as_deref())?;
    let contributors: Vec<Contributor> = actors
        .into_iter()
        .map(|(id, count)| Contributor { id, items: count })
        .collect();

    let title_for_log = args.title.clone();
    let release = Release {
        version: version.clone(),
        title: args.title,
        description: args.description,
        date: Utc::now().date_naive(),
        previous: previous.clone(),
        contributors,
        items: release_items,
    };

    // Show preview
    print_release(&release);

    // Confirm
    let hint = if args.bump.is_none() {
        " (use `joy release create minor` or `major` for other bumps)".to_string()
    } else {
        String::new()
    };
    print!("\nCreate release {}?{} [Y/n] ", version, hint);
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("y") {
        println!("Aborted.");
        return Ok(());
    }

    // Write YAML
    releases::save_release(&root, acronym, &release)?;
    println!(
        "Release saved to .joy/releases/{}-{}.yaml",
        acronym, version
    );

    // Log event
    event_log::log_event(
        &root,
        event_log::EventType::ReleaseCreated,
        &version,
        title_for_log.as_deref(),
    );

    Ok(())
}

fn show(args: ShowArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;
    let project = store::load_project(&root)?;
    let acronym = project.acronym.as_deref().unwrap_or("JOY");

    match args.version {
        Some(version) => {
            let release = releases::load_release(&root, acronym, &version)?;
            print_release(&release);
        }
        None => {
            // Preview: show what the next release would contain
            let cutoff = event_log::last_release_timestamp(&root)?;
            let closed_ids = event_log::closed_item_ids_since(&root, cutoff.as_deref())?;

            if closed_ids.is_empty() {
                println!("No items closed since last release.");
                return Ok(());
            }

            let previous = releases::latest_version(&root)?;
            let prev_str = previous.as_deref().unwrap_or("(none)");

            println!(
                "{} (preview, {} since {})",
                color::heading("Next release"),
                closed_ids.len(),
                prev_str
            );
            println!("{}", color::label(&"-".repeat(60)));

            let all_items = items::load_items(&root)?;
            print_items_grouped(&closed_ids, &all_items);

            let actors = event_log::actors_since(&root, cutoff.as_deref())?;
            if !actors.is_empty() {
                println!("\n{}", color::label("Contributors:"));
                for (actor, count) in &actors {
                    println!("  {} ({} events)", actor, count);
                }
            }
        }
    }

    Ok(())
}

fn ls() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let all_releases = releases::load_releases(&root)?;

    if all_releases.is_empty() {
        println!("No releases yet. Create one with: joy release create patch");
        return Ok(());
    }

    println!("{}", color::label(&"-".repeat(60)));
    println!(
        "{:<12} {:<12} {:>6}  {}",
        color::label("VERSION"),
        color::label("DATE"),
        color::label("ITEMS"),
        color::label("TITLE"),
    );
    println!("{}", color::label(&"-".repeat(60)));

    for release in &all_releases {
        let title = release.title.as_deref().unwrap_or("");
        println!(
            "{:<12} {:<12} {:>6}  {}",
            color::id(&release.version),
            release.date,
            release.items.total(),
            title,
        );
    }

    println!(
        "\n{}",
        color::label(&format!("{} release(s)", all_releases.len()))
    );

    Ok(())
}

fn terminal_width() -> usize {
    #[cfg(feature = "tui")]
    {
        if let Ok((cols, _)) = crossterm::terminal::size() {
            return cols as usize;
        }
    }
    std::env::var("COLUMNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(80)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    if max <= 3 {
        return ".".repeat(max);
    }
    format!("{}...", &s[..max - 3])
}

fn print_release(release: &Release) {
    let term_w = terminal_width();
    let title_str = release
        .title
        .as_deref()
        .map(|t| format!(" -- {t}"))
        .unwrap_or_default();
    println!(
        "{}{} ({})",
        color::heading(&release.version),
        title_str,
        release.date
    );
    println!("{}", color::label(&"-".repeat(term_w.min(60))));

    if let Some(ref desc) = release.description {
        println!("{desc}\n");
    }

    if !release.contributors.is_empty() {
        println!("{}", color::label("Contributors:"));
        for c in &release.contributors {
            println!("  {} ({} items)", c.id, c.items);
        }
        println!();
    }

    // Item ID (e.g. "JOY-0025") = ~8 chars + "  " prefix = 10, leave rest for title
    let title_max = term_w.saturating_sub(12);

    let type_groups: &[(&str, &[ReleaseItem])] = &[
        ("Epics", &release.items.epics),
        ("Stories", &release.items.stories),
        ("Tasks", &release.items.tasks),
        ("Bugs", &release.items.bugs),
        ("Reworks", &release.items.reworks),
        ("Decisions", &release.items.decisions),
        ("Ideas", &release.items.ideas),
    ];

    let total: usize = type_groups.iter().map(|(_, items)| items.len()).sum();

    for (label, items) in type_groups {
        if items.is_empty() {
            continue;
        }
        println!("{}:", color::label(label));
        for item in *items {
            println!(
                "  {} {}",
                color::id(&item.id),
                truncate(&item.title, title_max)
            );
        }
    }

    if total > 0 {
        println!("\n{}", color::label(&format!("{} item(s)", total)));
    }
}

fn print_items_grouped(item_ids: &[String], all_items: &[joy_core::model::item::Item]) {
    let term_w = terminal_width();
    let title_max = term_w.saturating_sub(12);

    let type_order = [
        (ItemType::Epic, "Epics"),
        (ItemType::Story, "Stories"),
        (ItemType::Task, "Tasks"),
        (ItemType::Bug, "Bugs"),
        (ItemType::Rework, "Reworks"),
        (ItemType::Decision, "Decisions"),
        (ItemType::Idea, "Ideas"),
    ];

    for (item_type, label) in &type_order {
        let group: Vec<_> = item_ids
            .iter()
            .filter_map(|id| all_items.iter().find(|i| &i.id == id))
            .filter(|i| &i.item_type == item_type)
            .collect();

        if group.is_empty() {
            continue;
        }

        println!("\n{}:", color::label(label));
        for item in &group {
            println!(
                "  {} {}",
                color::id(&item.id),
                truncate(&item.title, title_max)
            );
        }
    }
}
