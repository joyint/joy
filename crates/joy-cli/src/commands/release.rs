// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::io::Write;

use anyhow::Result;
use chrono::Utc;

use joy_core::event_log;
use joy_core::items;
use joy_core::model::item::{self, ItemType};
use joy_core::model::release::{Bump, Contributor, Release, ReleaseItem, ReleaseItems};
use joy_core::releases;
use joy_core::store;
use joy_core::vcs::{self, Vcs};

use crate::color;
use crate::forge;
use crate::version_bump;

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

    /// Full release: bump versions, git commit/tag/push, forge release
    #[arg(long)]
    full: bool,
}

#[derive(clap::Args)]
struct ShowArgs {
    /// Version to show (omit for next-release preview)
    version: Option<String>,

    /// Output as Markdown (for git tags and GitHub Releases)
    #[arg(long)]
    markdown: bool,
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
        println!("No items closed since last release.");
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

    // Build contributor list from events on the release items only
    let actors = event_log::actors_for_items(&root, &closed_ids)?;
    let contributors: Vec<Contributor> = actors
        .into_iter()
        .map(|a| Contributor {
            id: a.id,
            events: a.events,
            items: a.items,
        })
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
    print!("\nCreate release {}?{} [y/N] ", version, hint);
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    if !trimmed.eq_ignore_ascii_case("y") {
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

    // --full: version bump, git commit/tag/push, forge release
    if args.full {
        full_release(&root, &version, &release, &project)?;
    }

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
            if args.markdown {
                print_release_markdown(&release);
            } else {
                print_release(&release);
            }
        }
        None => {
            // Preview: show what the next release would contain
            let cutoff = event_log::last_release_timestamp(&root)?;
            let closed_ids = event_log::closed_item_ids_since(&root, cutoff.as_deref())?;

            if closed_ids.is_empty() {
                println!("No items closed since last release.");
                std::process::exit(1);
            }

            let previous = releases::latest_version(&root)?;
            let prev_str = previous.as_deref().unwrap_or("(none)");

            let header_text = format!(
                "Next release (preview, {} since {})",
                closed_ids.len(),
                prev_str
            );
            println!("{}", color::header(&header_text));

            let all_items = items::load_items(&root)?;
            print_items_grouped(&closed_ids, &all_items);

            let actors = event_log::actors_for_items(&root, &closed_ids)?;
            if !actors.is_empty() {
                println!("\n{}", color::label("Contributors:"));
                for a in &actors {
                    println!("  {} ({} events on {} items)", a.id, a.events, a.items);
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

    println!("{}", color::label(&"-".repeat(color::terminal_width())));
    println!(
        "{:<12} {:<12} {:>6}  {}",
        color::label("VERSION"),
        color::label("DATE"),
        color::label("ITEMS"),
        color::label("TITLE"),
    );
    println!("{}", color::label(&"-".repeat(color::terminal_width())));

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

    println!("{}", color::label(&"-".repeat(color::terminal_width())));
    println!(
        "{}",
        color::label(&color::plural(all_releases.len(), "release"))
    );

    Ok(())
}

fn full_release(
    root: &std::path::Path,
    version: &str,
    release: &Release,
    project: &joy_core::model::project::Project,
) -> Result<()> {
    let git = vcs::default_vcs();

    // Check git version
    git.check_version()?;

    // Version bump (if version-files configured)
    let version_files = read_version_files(root);
    let semver = version.strip_prefix('v').unwrap_or(version);
    if !version_files.is_empty() {
        let results = version_bump::bump_all(root, &version_files, semver)?;
        for r in &results {
            let rel_path = r.path.strip_prefix(root).unwrap_or(&r.path);
            println!("  {} -> {}", rel_path.display(), r.new_version);
        }
    }

    // Regenerate Cargo.lock after version bump
    if root.join("Cargo.lock").is_file() {
        let status = std::process::Command::new("cargo")
            .args(["generate-lockfile"])
            .current_dir(root)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if status.is_err() || !status.unwrap().success() {
            eprintln!("Warning: failed to update Cargo.lock");
        }
    }

    // Git: add, commit, tag, push
    git.add_all(root)?;
    git.commit(root, &format!("bump to {version} [no-item]"))?;

    // Annotated tag with markdown release notes
    let markdown_notes = render_release_markdown(release);
    git.tag_annotated(root, version, &markdown_notes)?;

    let remote = git.default_remote(root)?;
    git.push(root, &remote)?;
    git.push_tag(root, &remote, version)?;
    println!("Released {version}");

    // Forge release (optional, with confirmation)
    let forge_impl = forge::from_config(project.forge.as_deref());
    if project.forge.is_some() && project.forge.as_deref() != Some("none") {
        print!("Create forge release? [y/N] ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if input.trim().eq_ignore_ascii_case("y") {
            let title = release
                .title
                .as_deref()
                .map(|t| format!("{version} -- {t}"))
                .unwrap_or_else(|| version.to_string());
            match forge_impl.create_release(root, version, &title, &markdown_notes)? {
                Some(url) => println!("Forge release created: {url}"),
                None => println!("Forge release skipped."),
            }
        }
    }

    Ok(())
}

/// Read release.version-files from project.yaml as raw YAML.
fn read_version_files(root: &std::path::Path) -> Vec<version_bump::VersionFile> {
    let project_path = store::joy_dir(root).join(store::PROJECT_FILE);
    let content = match std::fs::read_to_string(&project_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let doc: serde_json::Value = match serde_yaml_ng::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let files = match doc.get("release").and_then(|r| r.get("version-files")) {
        Some(serde_json::Value::Array(arr)) => arr,
        _ => return Vec::new(),
    };
    files
        .iter()
        .filter_map(|entry| {
            let path = entry.get("path")?.as_str()?;
            let key = entry.get("key")?.as_str()?;
            Some(version_bump::VersionFile {
                path: path.to_string(),
                key: key.to_string(),
            })
        })
        .collect()
}

/// Render release as markdown string (for tag body and forge notes).
fn render_release_markdown(release: &Release) -> String {
    let mut out = String::new();
    let title_str = release
        .title
        .as_deref()
        .map(|t| format!(" -- {t}"))
        .unwrap_or_default();
    out.push_str(&format!("# {}{}\n\n", release.version, title_str));
    out.push_str(&format!("**Date:** {}\n", release.date));
    if let Some(ref prev) = release.previous {
        out.push_str(&format!("**Previous:** {prev}\n"));
    }
    if let Some(ref desc) = release.description {
        out.push_str(&format!("\n{desc}\n"));
    }
    if !release.contributors.is_empty() {
        out.push_str("\n## Contributors\n\n");
        for c in &release.contributors {
            out.push_str(&format!(
                "- {} ({} events on {} items)\n",
                c.id, c.events, c.items
            ));
        }
    }
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
        out.push_str(&format!("\n## {label}\n\n"));
        for ri in *items {
            let filename = item::item_filename(&ri.id, &ri.title);
            out.push_str(&format!(
                "- [{}](.joy/items/{}) {}\n",
                ri.id, filename, ri.title
            ));
        }
    }
    if total > 0 {
        out.push_str(&format!("\n---\n*{}*\n", color::plural(total, "item")));
    }
    out
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
    let w = color::terminal_width();
    let title_str = release
        .title
        .as_deref()
        .map(|t| format!(" -- {t}"))
        .unwrap_or_default();
    let header_text = format!("{}{} ({})", release.version, title_str, release.date);
    println!("{}", color::header(&header_text));

    if let Some(ref desc) = release.description {
        println!("{desc}\n");
    }

    if !release.contributors.is_empty() {
        println!("{}", color::label("Contributors:"));
        for c in &release.contributors {
            println!("  {} ({} events on {} items)", c.id, c.events, c.items);
        }
        println!();
    }

    // Item ID (e.g. "JOY-0025") = ~8 chars + "  " prefix = 10, leave rest for title
    let title_max = w.saturating_sub(12);

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
        let mut stats: Vec<String> = Vec::new();
        for (label, items) in type_groups {
            if !items.is_empty() {
                // label is already plural ("Stories"), derive singular by trimming "s"
                let singular = label.trim_end_matches('s').to_lowercase();
                stats.push(color::plural(items.len(), &singular));
            }
        }
        println!("{}", color::label(&"-".repeat(w)));
        println!(
            "{}",
            color::label(&format!(
                "{} · {}",
                color::plural(total, "item"),
                stats.join(" · ")
            ))
        );
    }
}

fn print_items_grouped(item_ids: &[String], all_items: &[joy_core::model::item::Item]) {
    let w = color::terminal_width();
    let title_max = w.saturating_sub(12);

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

fn item_link(ri: &ReleaseItem) -> String {
    let filename = item::item_filename(&ri.id, &ri.title);
    format!("[{}](.joy/items/{})", ri.id, filename)
}

fn print_release_markdown(release: &Release) {
    let title_str = release
        .title
        .as_deref()
        .map(|t| format!(" -- {t}"))
        .unwrap_or_default();
    println!("# {}{}", release.version, title_str);
    println!();
    println!("**Date:** {}", release.date);

    if let Some(ref prev) = release.previous {
        println!("**Previous:** {prev}");
    }

    if let Some(ref desc) = release.description {
        println!();
        println!("{desc}");
    }

    if !release.contributors.is_empty() {
        println!();
        println!("## Contributors");
        println!();
        for c in &release.contributors {
            println!("- {} ({} events on {} items)", c.id, c.events, c.items);
        }
    }

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
        println!();
        println!("## {label}");
        println!();
        for ri in *items {
            println!("- {} {}", item_link(ri), ri.title);
        }
    }

    if total > 0 {
        println!();
        println!("---");
        println!("*{}*", color::plural(total, "item"));
    }
}
