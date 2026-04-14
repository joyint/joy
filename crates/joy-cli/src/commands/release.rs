// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::io::Write;

use anyhow::Result;
use chrono::Utc;

use joy_core::context::Context;
use joy_core::event_log;
use joy_core::guard::Action;
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
    /// Step 1: patch version numbers in configured files
    Bump(BumpArgs),
    /// Step 2: write release record, commit, tag (local only)
    Record(RecordArgs),
    /// Step 3: push commits + tag, create forge release
    Publish(PublishArgs),
    /// Show a release or preview the next one
    Show(ShowArgs),
    /// List all releases
    Ls,
}

#[derive(clap::Args)]
struct BumpArgs {
    /// Version bump: patch (default), minor, major, or an explicit X.Y.Z
    bump: Option<String>,
}

#[derive(clap::Args)]
struct RecordArgs {
    /// Version bump: patch (default), minor, major, or an explicit X.Y.Z.
    /// Must match what was used for `joy release bump`.
    bump: Option<String>,

    /// Release title
    #[arg(long)]
    title: Option<String>,

    /// Release description
    #[arg(long)]
    description: Option<String>,
}

#[derive(clap::Args)]
struct PublishArgs {
    /// Version to publish. Defaults to the current tag on HEAD.
    version: Option<String>,
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
        ReleaseCommand::Bump(args) => bump(args),
        ReleaseCommand::Record(args) => record(args),
        ReleaseCommand::Publish(args) => publish(args),
        ReleaseCommand::Show(args) => show(args),
        ReleaseCommand::Ls => ls(),
    }
}

/// Compute the new version from the bump argument and the previous
/// release (or latest tag). Deterministic: `bump` and `record` call
/// this with the same argument and land on the same version.
fn resolve_version(root: &std::path::Path, arg: Option<&str>) -> Result<(String, String)> {
    let previous = releases::latest_version(root)?.or_else(|| {
        joy_core::vcs::default_vcs()
            .latest_version_tag(root)
            .ok()
            .flatten()
    });
    let current = previous.as_deref().unwrap_or("v0.0.0").to_string();

    let next = match arg {
        Some(v) if looks_like_explicit(v) => {
            if v.starts_with('v') {
                v.to_string()
            } else {
                format!("v{v}")
            }
        }
        Some(b) => {
            let bump: Bump = b.parse().map_err(|e: String| anyhow::anyhow!("{}", e))?;
            joy_core::model::release::bump_version(&current, bump)
        }
        None => {
            let bump: Bump = "patch"
                .parse()
                .map_err(|e: String| anyhow::anyhow!("{}", e))?;
            joy_core::model::release::bump_version(&current, bump)
        }
    };
    Ok((current, next))
}

fn looks_like_explicit(s: &str) -> bool {
    matches!(s.chars().next(), Some(c) if c.is_ascii_digit()) || s.starts_with('v')
}

fn bump(args: BumpArgs) -> Result<()> {
    let ctx = Context::load()?;
    ctx.enforce(&Action::CreateRelease, "release")?;

    let (current, next) = resolve_version(&ctx.root, args.bump.as_deref())?;
    let current_semver = current.strip_prefix('v').unwrap_or(&current);
    let next_semver = next.strip_prefix('v').unwrap_or(&next);

    let version_files = read_version_files(&ctx.root);
    if version_files.is_empty() {
        println!("No release.version-files configured in project.yaml -- nothing to patch.");
        println!("Next version will be {next}.");
        return Ok(());
    }

    let results = version_bump::bump_all(&ctx.root, &version_files, current_semver, next_semver)?;

    println!("{} -> {}", color::label(&current), color::id(&next));
    let mut total = 0usize;
    for r in &results {
        let rel = r.path.strip_prefix(&ctx.root).unwrap_or(&r.path);
        let marker = if r.replacements == 0 { "!" } else { " " };
        println!(
            "  {marker} {} ({} replacement{})",
            rel.display(),
            r.replacements,
            if r.replacements == 1 { "" } else { "s" }
        );
        total += r.replacements;
    }
    if total == 0 {
        anyhow::bail!(
            "no occurrences of {current_semver} found in configured files\n  = help: check release.version-files and the old version string",
        );
    }
    println!(
        "\nNext: run lockfile refresh if needed, then `joy release record {}`.",
        args.bump.as_deref().unwrap_or("patch")
    );
    Ok(())
}

fn record(args: RecordArgs) -> Result<()> {
    let ctx = Context::load()?;
    ctx.enforce(&Action::CreateRelease, "release")?;

    let project = store::load_project(&ctx.root)?;
    let acronym = project.acronym.as_deref().unwrap_or("JOY");

    let (previous, version) = resolve_version(&ctx.root, args.bump.as_deref())?;
    let previous_opt = if previous == "v0.0.0" {
        None
    } else {
        Some(previous)
    };

    if releases::load_release(&ctx.root, acronym, &version).is_ok() {
        anyhow::bail!("Release {} already exists", version);
    }

    let cutoff = event_log::last_release_timestamp(&ctx.root)?;
    let closed_ids = event_log::closed_item_ids_since(&ctx.root, cutoff.as_deref())?;

    if closed_ids.is_empty() {
        println!("No items closed since last release.");
        return Ok(());
    }

    let all_items = items::load_items(&ctx.root)?;
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

    let actors = event_log::actors_for_items(&ctx.root, &closed_ids)?;
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
        previous: previous_opt,
        contributors,
        items: release_items,
    };

    print_release(&release);

    print!("\nRecord release {}? [y/N] ", version);
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    if !input.trim().eq_ignore_ascii_case("y") {
        println!("Aborted.");
        return Ok(());
    }

    releases::save_release(&ctx.root, acronym, &release)?;
    println!(
        "Release saved to .joy/releases/{}-{}.yaml",
        acronym, version
    );

    let log_user = ctx.log_user();
    event_log::log_event_as(
        &ctx.root,
        event_log::EventType::ReleaseCreated,
        &version,
        title_for_log.as_deref(),
        &log_user,
    );

    // Git: add + commit + local tag. No push, no forge call.
    let git = vcs::default_vcs();
    git.check_version()?;
    git.add_all(&ctx.root)?;
    git.commit(&ctx.root, &format!("bump to {version} [no-item]"))?;
    let markdown_notes = render_release_markdown(&release);
    git.tag_annotated(&ctx.root, &version, &markdown_notes)?;
    println!("Tag {version} created locally. Next: `joy release publish`.");
    Ok(())
}

fn publish(args: PublishArgs) -> Result<()> {
    let ctx = Context::load()?;
    ctx.enforce(&Action::CreateRelease, "release")?;

    let project = store::load_project(&ctx.root)?;
    let acronym = project.acronym.as_deref().unwrap_or("JOY");

    let git = vcs::default_vcs();
    git.check_version()?;

    let version = match args.version {
        Some(v) if v.starts_with('v') => v,
        Some(v) => format!("v{v}"),
        None => git
            .latest_version_tag(&ctx.root)
            .ok()
            .flatten()
            .ok_or_else(|| anyhow::anyhow!("no local tag to publish; pass an explicit version"))?,
    };

    let release = releases::load_release(&ctx.root, acronym, &version).map_err(|_| {
        anyhow::anyhow!("no release record for {version} (run `joy release record` first)")
    })?;

    let remote = git.default_remote(&ctx.root)?;
    println!("Pushing to {remote}...");
    git.push(&ctx.root, &remote)?;
    git.push_tag(&ctx.root, &remote, &version)?;
    println!("Pushed {version} to {remote}.");

    let forge_impl = forge::from_config(project.forge.as_deref());
    if project.forge.as_deref().is_none() || project.forge.as_deref() == Some("none") {
        println!("No forge configured; publish done.");
        return Ok(());
    }

    let markdown_notes = render_release_markdown(&release);
    let title = release
        .title
        .as_deref()
        .map(|t| format!("{version} - {t}"))
        .unwrap_or_else(|| version.clone());
    match forge_impl.create_release(&ctx.root, &version, &title, &markdown_notes)? {
        Some(url) => println!("Forge release created: {url}"),
        None => println!("Forge release skipped."),
    }
    Ok(())
}

fn show(args: ShowArgs) -> Result<()> {
    let ctx = Context::load()?;
    let project = store::load_project(&ctx.root)?;
    let acronym = project.acronym.as_deref().unwrap_or("JOY");

    match args.version {
        Some(version) => {
            let release = releases::load_release(&ctx.root, acronym, &version)?;
            if args.markdown {
                print_release_markdown(&release);
            } else {
                print_release(&release);
            }
        }
        None => {
            let cutoff = event_log::last_release_timestamp(&ctx.root)?;
            let closed_ids = event_log::closed_item_ids_since(&ctx.root, cutoff.as_deref())?;

            if closed_ids.is_empty() {
                println!("No items closed since last release.");
                std::process::exit(1);
            }

            let previous = releases::latest_version(&ctx.root)?;
            let prev_str = previous.as_deref().unwrap_or("(none)");

            let header_text = format!(
                "Next release (preview, {} since {})",
                closed_ids.len(),
                prev_str
            );
            println!("{}", color::header(&header_text));

            let all_items = items::load_items(&ctx.root)?;
            print_items_grouped(&closed_ids, &all_items);

            let actors = event_log::actors_for_items(&ctx.root, &closed_ids)?;
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
    let ctx = Context::load()?;

    let all_releases = releases::load_releases(&ctx.root)?;

    if all_releases.is_empty() {
        println!("No releases yet. Create one with: joy release bump patch");
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

/// Read release.version-files from project.yaml as raw YAML.
/// Each entry is a path string or a mapping with a `path` field.
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
            if let Some(s) = entry.as_str() {
                return Some(version_bump::VersionFile {
                    path: s.to_string(),
                });
            }
            let path = entry.get("path")?.as_str()?;
            Some(version_bump::VersionFile {
                path: path.to_string(),
            })
        })
        .collect()
}

fn render_release_markdown(release: &Release) -> String {
    let mut out = String::new();
    let title_str = release
        .title
        .as_deref()
        .map(|t| format!(" - {t}"))
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
        .map(|t| format!(" - {t}"))
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
        .map(|t| format!(" - {t}"))
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
