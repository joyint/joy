// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::io::Write;

use anyhow::Result;
use chrono::Utc;

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
    /// Use the version currently written in the configured files as
    /// the baseline, instead of the latest release / git tag. Useful
    /// when a scaffolded template was published with a non-default
    /// version (e.g. Next.js projects start at 0.1.0) or when an
    /// out-of-band `npm version` ran. All files must agree.
    #[arg(long)]
    adopt: bool,
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

    /// Override the forge for this publish. Highest precedence:
    /// takes priority over `forge:` in project.yaml and over
    /// auto-detection. Use `none` to push the tag without creating
    /// a forge release.
    #[arg(long)]
    forge: Option<String>,
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
/// this with the same argument and land on the same version. When
/// `baseline_override` is set, that value replaces the ledger / tag
/// lookup (used by `joy release bump --adopt`).
fn resolve_version(
    root: &std::path::Path,
    arg: Option<&str>,
    baseline_override: Option<String>,
) -> Result<(String, String)> {
    let current = match baseline_override {
        Some(v) => {
            if v.starts_with('v') {
                v
            } else {
                format!("v{v}")
            }
        }
        None => {
            let previous = releases::latest_version(root)?.or_else(|| {
                joy_core::vcs::default_vcs()
                    .latest_version_tag(root)
                    .ok()
                    .flatten()
            });
            previous.as_deref().unwrap_or("v0.0.0").to_string()
        }
    };

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
    let ctx = crate::crypt_session::load_context(None)?;
    ctx.enforce(&Action::CreateRelease, "release")?;

    let version_files = read_version_files(&ctx.root);
    if version_files.is_empty() {
        let (_, next) = resolve_version(&ctx.root, args.bump.as_deref(), None)?;
        println!("No release.version-files configured in project.yaml -- nothing to patch.");
        println!("Next version will be {next}.");
        return Ok(());
    }

    let baseline_override = if args.adopt {
        Some(adopt_baseline(&ctx.root, &version_files)?)
    } else {
        None
    };

    let (current, next) = resolve_version(&ctx.root, args.bump.as_deref(), baseline_override)?;
    let current_semver = current.strip_prefix('v').unwrap_or(&current);
    let next_semver = next.strip_prefix('v').unwrap_or(&next);

    let results = version_bump::bump_all(&ctx.root, &version_files, current_semver, next_semver)?;

    println!("{} -> {}", color::label(&current), color::id(&next));
    let total: usize = results.iter().map(|r| r.replacements).sum();

    if total == 0 {
        return Err(version_mismatch_error(&ctx.root, &results, current_semver));
    }

    for r in &results {
        let rel = r.path.strip_prefix(&ctx.root).unwrap_or(&r.path);
        let marker = if r.replacements == 0 { "!" } else { " " };
        println!(
            "  {marker} {} ({} replacement{})",
            rel.display(),
            r.replacements,
            if r.replacements == 1 { "" } else { "s" }
        );
    }
    println!(
        "\nNext: run lockfile refresh if needed, then `joy release record {}`.",
        args.bump.as_deref().unwrap_or("patch")
    );
    Ok(())
}

/// Scan the configured files for the version they currently contain
/// (via `version_bump::detect_version`) and return that as the new
/// baseline for `--adopt`. All files must agree; files where detection
/// fails are listed but tolerated as long as at least one file
/// produces a version. Failure modes get their own clear error.
fn adopt_baseline(
    root: &std::path::Path,
    version_files: &[version_bump::VersionFile],
) -> Result<String> {
    let mut by_version: std::collections::BTreeMap<String, Vec<std::path::PathBuf>> =
        std::collections::BTreeMap::new();
    let mut undetectable: Vec<std::path::PathBuf> = Vec::new();

    for vf in version_files {
        let pattern = root.join(&vf.path);
        let paths: Vec<_> = glob::glob(&pattern.to_string_lossy())
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect();
        for path in paths {
            match version_bump::detect_version(&path) {
                Some(v) => by_version.entry(v).or_default().push(path),
                None => undetectable.push(path),
            }
        }
    }

    match by_version.len() {
        0 => {
            let mut msg =
                String::from("--adopt: could not detect a version in any configured file");
            for path in &undetectable {
                let rel = path.strip_prefix(root).unwrap_or(path);
                msg.push_str(&format!("\n  ! {}", rel.display()));
            }
            msg.push_str("\n  = help: pass an explicit X.Y.Z to bump instead");
            anyhow::bail!("{msg}")
        }
        1 => {
            let v = by_version.into_keys().next().unwrap();
            Ok(v)
        }
        _ => {
            let mut msg = String::from("--adopt: configured files disagree on the current version");
            for (v, files) in &by_version {
                for path in files {
                    let rel = path.strip_prefix(root).unwrap_or(path);
                    msg.push_str(&format!("\n  {} -> {}", rel.display(), v));
                }
            }
            msg.push_str("\n  = help: align the files manually or pass an explicit X.Y.Z");
            anyhow::bail!("{msg}")
        }
    }
}

/// Build the multi-line "version mismatch" error printed when
/// `joy release bump` finds zero matches. The block per file shows
/// what joy expected versus what it actually detected; the closing
/// section names the two recovery commands. Designed for narrow
/// terminals -- every emitted line stays short.
fn version_mismatch_error(
    root: &std::path::Path,
    results: &[version_bump::BumpResult],
    expected: &str,
) -> anyhow::Error {
    // Per-file diagnostic goes to stdout above the Error: line.
    let mut detected_any: Option<String> = None;
    println!();
    for r in results {
        let rel = r.path.strip_prefix(root).unwrap_or(&r.path);
        let detected = version_bump::detect_version(&r.path);
        if detected_any.is_none() {
            detected_any = detected.clone();
        }
        println!("  ! {}", rel.display());
        println!("      expected: {expected}");
        match detected {
            Some(v) => println!("      found:    {v}"),
            None => println!("      found:    (no version detected)"),
        }
    }
    println!();

    let n = results.len();
    let plural = if n == 1 { "" } else { "s" };
    let mut msg = format!("version mismatch ({n} of {n} file{plural})\n\nFix options:");
    msg.push_str("\n\n  joy release bump --adopt");
    msg.push_str("\n      adopt the file's detected version");
    if let Some(v) = detected_any {
        msg.push_str(&format!("\n\n  joy release record {v}"));
        msg.push_str("\n      skip bump, record at the detected version");
    } else {
        msg.push_str("\n\n  joy release record <X.Y.Z>");
        msg.push_str("\n      skip bump, record at an explicit version");
    }
    anyhow::anyhow!("{msg}")
}

fn record(args: RecordArgs) -> Result<()> {
    let ctx = crate::crypt_session::load_context(None)?;
    ctx.enforce(&Action::CreateRelease, "release")?;

    let project = store::load_project(&ctx.root)?;
    let acronym = project.acronym.as_deref().unwrap_or("JOY");

    let (previous, version) = resolve_version(&ctx.root, args.bump.as_deref(), None)?;
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
    let is_empty_release = closed_ids.is_empty();

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

    if is_empty_release {
        // No items closed since the previous release. We still record an
        // empty .yaml so that `joy release publish` (and downstream
        // release-all) is idempotent across submodules with nothing
        // closed; otherwise the absent record breaks the publish step
        // (JOY-0163-95).
        println!(
            "\nNo items closed since the previous release; recording empty release {version}."
        );
    } else {
        print!("\nRecord release {}? [y/N] ", version);
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(());
        }
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
        None,
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
    let ctx = crate::crypt_session::load_context(None)?;
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

    // Resolve the forge before pushing. Push + tag-push are not
    // trivially reversible, so we fail early if we can't decide which
    // forge to talk to. The publish step is still idempotent on retry
    // because gh release create dedupes by tag.
    let forge_choice = forge::resolve(&ctx.root, project.forge.as_deref(), args.forge.as_deref())?;
    if let Some(note) = &forge_choice.note {
        println!("{note}");
    }

    let remote = git.default_remote(&ctx.root)?;
    println!("Pushing to {remote}...");
    git.push(&ctx.root, &remote)?;
    git.push_tag(&ctx.root, &remote, &version)?;
    println!("Pushed {version} to {remote}.");

    let markdown_notes = render_release_markdown(&release);
    let title = release
        .title
        .as_deref()
        .map(|t| format!("{version} - {t}"))
        .unwrap_or_else(|| version.clone());
    match forge_choice
        .forge
        .create_release(&ctx.root, &version, &title, &markdown_notes)?
    {
        Some(url) => println!("Forge release created: {url}"),
        None => println!("Forge release skipped."),
    }
    Ok(())
}

fn show(args: ShowArgs) -> Result<()> {
    let ctx = crate::crypt_session::load_context(None)?;
    let project = store::load_project(&ctx.root)?;
    let acronym = project.acronym.as_deref().unwrap_or("JOY");

    match args.version {
        Some(version) => {
            let release = releases::load_release(&ctx.root, acronym, &version)?;
            if crate::output::is_json() {
                return crate::output::emit(&release);
            }
            if args.markdown {
                print_release_markdown(&release);
            } else {
                print_release(&release);
            }
        }
        None => {
            let cutoff = event_log::last_release_timestamp(&ctx.root)?;
            let closed_ids = event_log::closed_item_ids_since(&ctx.root, cutoff.as_deref())?;

            let previous = releases::latest_version(&ctx.root)?;

            if crate::output::is_json() {
                return crate::output::emit(ReleasePreviewPayload {
                    previous_version: previous.clone(),
                    closed_item_ids: closed_ids.clone(),
                });
            }

            if closed_ids.is_empty() {
                println!("No items closed since last release.");
                std::process::exit(1);
            }

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
    let ctx = crate::crypt_session::load_context(None)?;

    let all_releases = releases::load_releases(&ctx.root)?;

    if crate::output::is_json() {
        return crate::output::emit(ReleaseListPayload {
            total: all_releases.len(),
            releases: all_releases,
        });
    }

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

#[derive(serde::Serialize)]
struct ReleaseListPayload {
    total: usize,
    releases: Vec<joy_core::model::Release>,
}

#[derive(serde::Serialize)]
struct ReleasePreviewPayload {
    previous_version: Option<String>,
    closed_item_ids: Vec<String>,
}
