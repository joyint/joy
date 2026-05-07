// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! `joy update` -- swap the binary (when this build was distributed via
//! cargo-dist) and bring the current repo's joy-managed state in line
//! with the new binary. See JOY-0164-B5 / JOY-0169-6A.
//!
//! Per-artefact logic lives in [`crate::update_registry`]. This module
//! is a thin orchestrator: iterate the registry, format rows joy-style,
//! and stitch the binary self-update around it.
//!
//! The binary swap is receipt-gated by `axoupdater`: only cargo-dist
//! installer-managed binaries carry the install receipt, so brew /
//! cargo-install / distro-package builds skip the swap with a clear
//! message instead of clobbering a foreign-managed binary.

use std::path::Path;

use anyhow::Result;
use axoupdater::AxoUpdater;
use clap::Args;

use joy_core::store;

use crate::color;
use crate::update_registry::{self, RowMark, UpdateItem};

const PKG_NAME: &str = "joy";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Section ordering used in the displayed output. Items are grouped by
/// their `section()` value; within a section the order is the order in
/// which they appear in [`update_registry::all`].
const SECTION_ORDER: &[&str] = &[
    update_registry::SECTION_REPO,
    update_registry::SECTION_GIT,
    update_registry::SECTION_EMBEDDED,
    update_registry::SECTION_AUTH,
    update_registry::SECTION_AI,
];

#[derive(Args)]
#[command(after_help = "\
Updates the joy binary (when distributed via cargo-dist / GitHub
Releases) and synchronises the current repo's joy-managed state.

Examples:
  joy update                Update binary, then sync this repo
  joy update --check        Check whether anything would change
  joy update --no-binary    Sync only, do not touch the binary")]
pub struct UpdateArgs {
    /// Only check; do not modify anything. Exit 2 if anything would change.
    #[arg(long)]
    pub check: bool,

    /// Skip the binary self-update; only run the in-repo sync.
    #[arg(long = "no-binary")]
    pub no_binary: bool,
}

pub fn run(args: UpdateArgs) -> Result<()> {
    if args.check {
        if crate::output::is_json() {
            return run_check_json();
        }
        return run_check();
    }
    if crate::output::is_json() {
        return run_update_json(args);
    }

    let cwd = std::env::current_dir()?;
    let project_root = store::find_project_root(&cwd);

    println!("{}", color::header("Joy Update"));
    println!();

    println!("{}", color::section("Binary"));
    if args.no_binary {
        println!(
            "  {}{:<24} {}",
            color::empty_mark(),
            "binary",
            color::inactive("skipped (--no-binary)")
        );
    } else {
        let (mark, status) = swap_binary_status();
        println!("  {mark}{:<24} {}", "binary", status);
    }

    if let Some(root) = project_root {
        let changed = run_full_sync(&root);
        println!();
        let summary = match changed {
            0 => "Joy update complete -- nothing to refresh.".to_string(),
            1 => "Joy update complete -- 1 artefact refreshed.".to_string(),
            n => format!("Joy update complete -- {n} artefacts refreshed."),
        };
        println!("{}", color::footer(&summary));
    } else {
        println!();
        println!(
            "{}",
            color::footer("Not inside a Joy project; in-repo sync skipped.")
        );
    }

    Ok(())
}

/// Iterate the update registry, refreshing every item. Prints joy-style
/// rows ONLY for items that actually changed (terse by design); section
/// headers are printed lazily when the first changed row appears.
/// Returns the total number of refreshed artefacts.
pub(crate) fn run_full_sync(root: &Path) -> u32 {
    let items = update_registry::all();
    let mut changed = 0u32;
    for section in SECTION_ORDER {
        let mut header_printed = false;
        for item in items.iter().filter(|i| &i.section() == section) {
            let rows = match item.refresh(root) {
                Ok(rs) => rs,
                Err(e) => {
                    eprintln!("warning: refresh of {} failed: {e}", item.section());
                    continue;
                }
            };
            for row in rows {
                if let Some(action) = row.action {
                    if !header_printed {
                        println!("{}", color::section(section));
                        header_printed = true;
                    }
                    println!(
                        "  {}{:<24} {}",
                        color::check_mark(),
                        row.name,
                        color::success(action)
                    );
                    changed += 1;
                }
            }
        }
        if header_printed {
            println!();
        }
    }
    // Trim the trailing blank line we always add after the last section.
    changed
}

fn run_check() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let mut stale = false;

    println!("{}", color::header("Joy update check"));
    println!();

    println!("{}", color::section("Binary"));
    let mut updater = AxoUpdater::new_for(PKG_NAME);
    if updater.load_receipt().is_err() {
        println!(
            "  {}install receipt missing {}",
            color::empty_mark(),
            color::inactive("(managed by another installer)")
        );
    } else if updater.is_update_needed_sync().unwrap_or(false) {
        stale = true;
        println!(
            "  {}update available {}",
            color::warn_mark(),
            color::warning(&format!("(current {CURRENT_VERSION})"))
        );
    } else {
        println!(
            "  {}up to date {}",
            color::check_mark(),
            color::inactive(&format!("({CURRENT_VERSION})"))
        );
    }

    let Some(root) = store::find_project_root(&cwd) else {
        println!();
        println!("Not inside a Joy project; skipping in-repo checks.");
        if stale {
            std::process::exit(2);
        }
        return Ok(());
    };

    let items = update_registry::all();
    for section in SECTION_ORDER {
        let section_items: Vec<&Box<dyn UpdateItem>> =
            items.iter().filter(|i| &i.section() == section).collect();
        if section_items.is_empty() {
            continue;
        }
        println!();
        println!("{}", color::section(section));
        for item in section_items {
            let rows = match item.check(&root) {
                Ok(rs) => rs,
                Err(e) => {
                    stale = true;
                    println!("  {}error: {e}", color::warn_mark());
                    continue;
                }
            };
            for row in rows {
                let (mark, detail) = match row.mark {
                    RowMark::Ok => (color::check_mark(), color::inactive(&row.detail)),
                    RowMark::Stale => {
                        stale = true;
                        (color::warn_mark(), color::warning(&row.detail))
                    }
                    RowMark::Info => (color::empty_mark(), color::inactive(&row.detail)),
                };
                println!("  {mark}{:<24} {}", row.name, detail);
            }
        }
    }

    println!();
    if stale {
        println!(
            "{}",
            color::footer(&format!(
                "Stale items found -- run {} to refresh.",
                color::label("joy update")
            ))
        );
        std::process::exit(2);
    } else {
        println!("{}", color::footer("All Joy-managed state is up to date."));
    }
    Ok(())
}

// -- JSON output ---------------------------------------------------------

#[derive(serde::Serialize)]
struct CheckPayload {
    binary: BinaryStatus,
    repo_root: Option<String>,
    sections: Vec<CheckSection>,
    has_issues: bool,
}

#[derive(serde::Serialize)]
struct CheckSection {
    name: &'static str,
    rows: Vec<CheckRowJson>,
}

#[derive(serde::Serialize)]
struct CheckRowJson {
    name: String,
    /// `"ok"`, `"stale"`, or `"info"`.
    status: &'static str,
    detail: String,
}

#[derive(serde::Serialize)]
struct UpdatePayload {
    binary: BinaryAction,
    repo_root: Option<String>,
    sections: Vec<UpdateSection>,
    refreshed_count: u32,
}

#[derive(serde::Serialize)]
struct UpdateSection {
    name: &'static str,
    rows: Vec<UpdateRowJson>,
}

#[derive(serde::Serialize)]
struct UpdateRowJson {
    name: String,
    /// `Some(verb)` if the artefact was changed; `None` if already up
    /// to date.
    action: Option<&'static str>,
}

#[derive(serde::Serialize)]
struct BinaryStatus {
    /// `"up_to_date"`, `"update_available"`, or `"unmanaged"`.
    state: &'static str,
    current_version: &'static str,
}

#[derive(serde::Serialize)]
struct BinaryAction {
    /// `"updated"`, `"already_current"`, `"unmanaged"`, `"skipped"`,
    /// or `"failed"`.
    state: &'static str,
    current_version: &'static str,
    /// Set on `"failed"`.
    error: Option<String>,
    /// Set on `"updated"`.
    old_version: Option<String>,
    /// Set on `"updated"`.
    new_version: Option<String>,
}

fn run_check_json() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let mut updater = AxoUpdater::new_for(PKG_NAME);
    let binary = if updater.load_receipt().is_err() {
        BinaryStatus {
            state: "unmanaged",
            current_version: CURRENT_VERSION,
        }
    } else if updater.is_update_needed_sync().unwrap_or(false) {
        BinaryStatus {
            state: "update_available",
            current_version: CURRENT_VERSION,
        }
    } else {
        BinaryStatus {
            state: "up_to_date",
            current_version: CURRENT_VERSION,
        }
    };
    let mut has_issues = matches!(binary.state, "update_available");
    let mut sections: Vec<CheckSection> = Vec::new();
    let mut repo_root: Option<String> = None;

    if let Some(root) = store::find_project_root(&cwd) {
        repo_root = Some(root.display().to_string());
        let items = update_registry::all();
        for section in SECTION_ORDER {
            let mut rows: Vec<CheckRowJson> = Vec::new();
            for item in items.iter().filter(|i| &i.section() == section) {
                match item.check(&root) {
                    Ok(rs) => {
                        for row in rs {
                            let status = match row.mark {
                                RowMark::Ok => "ok",
                                RowMark::Stale => {
                                    has_issues = true;
                                    "stale"
                                }
                                RowMark::Info => "info",
                            };
                            rows.push(CheckRowJson {
                                name: row.name,
                                status,
                                detail: row.detail,
                            });
                        }
                    }
                    Err(e) => {
                        has_issues = true;
                        rows.push(CheckRowJson {
                            name: "error".into(),
                            status: "stale",
                            detail: e.to_string(),
                        });
                    }
                }
            }
            if !rows.is_empty() {
                sections.push(CheckSection {
                    name: section,
                    rows,
                });
            }
        }
    }

    crate::output::emit(CheckPayload {
        binary,
        repo_root,
        sections,
        has_issues,
    })?;
    if has_issues {
        std::process::exit(2);
    }
    Ok(())
}

fn run_update_json(args: UpdateArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let project_root = store::find_project_root(&cwd);

    let binary = if args.no_binary {
        BinaryAction {
            state: "skipped",
            current_version: CURRENT_VERSION,
            error: None,
            old_version: None,
            new_version: None,
        }
    } else {
        let mut updater = AxoUpdater::new_for(PKG_NAME);
        if updater.load_receipt().is_err() {
            BinaryAction {
                state: "unmanaged",
                current_version: CURRENT_VERSION,
                error: None,
                old_version: None,
                new_version: None,
            }
        } else {
            match updater.run_sync() {
                Ok(Some(result)) => BinaryAction {
                    state: "updated",
                    current_version: CURRENT_VERSION,
                    error: None,
                    old_version: Some(format!("{:?}", result.old_version)),
                    new_version: Some(format!("{:?}", result.new_version)),
                },
                Ok(None) => BinaryAction {
                    state: "already_current",
                    current_version: CURRENT_VERSION,
                    error: None,
                    old_version: None,
                    new_version: None,
                },
                Err(e) => BinaryAction {
                    state: "failed",
                    current_version: CURRENT_VERSION,
                    error: Some(e.to_string()),
                    old_version: None,
                    new_version: None,
                },
            }
        }
    };

    let mut sections: Vec<UpdateSection> = Vec::new();
    let mut refreshed_count = 0u32;
    let mut repo_root: Option<String> = None;

    if let Some(root) = project_root {
        repo_root = Some(root.display().to_string());
        let items = update_registry::all();
        for section in SECTION_ORDER {
            let mut rows: Vec<UpdateRowJson> = Vec::new();
            for item in items.iter().filter(|i| &i.section() == section) {
                if let Ok(rs) = item.refresh(&root) {
                    for row in rs {
                        if let Some(action) = row.action {
                            refreshed_count += 1;
                            rows.push(UpdateRowJson {
                                name: row.name,
                                action: Some(action),
                            });
                        }
                    }
                }
            }
            if !rows.is_empty() {
                sections.push(UpdateSection {
                    name: section,
                    rows,
                });
            }
        }
    }

    crate::output::emit(UpdatePayload {
        binary,
        repo_root,
        sections,
        refreshed_count,
    })?;
    Ok(())
}

/// One-line binary swap result: (mark, status detail).
fn swap_binary_status() -> (&'static str, String) {
    let mut updater = AxoUpdater::new_for(PKG_NAME);
    if updater.load_receipt().is_err() {
        return (
            color::empty_mark(),
            color::inactive(&format!("managed by another installer ({CURRENT_VERSION})")),
        );
    }
    match updater.run_sync() {
        Ok(Some(result)) => (
            color::check_mark(),
            color::success(&format!(
                "updated {:?} -> {:?}",
                result.old_version, result.new_version
            )),
        ),
        Ok(None) => (
            color::check_mark(),
            color::inactive(&format!("up to date ({CURRENT_VERSION})")),
        ),
        Err(e) => (color::warn_mark(), color::warning(&format!("failed: {e}"))),
    }
}
