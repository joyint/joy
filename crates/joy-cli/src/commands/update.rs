// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! `joy update` -- swap the binary (when this build was distributed via
//! cargo-dist) and bring the current repo's joy-managed state in line
//! with the new binary. See JOY-0164-B5.
//!
//! The binary swap is receipt-gated by `axoupdater`: only cargo-dist
//! installer-managed binaries carry the install receipt, so brew /
//! cargo-install / distro-package builds skip the swap with a clear
//! message instead of clobbering a foreign-managed binary.

use std::path::Path;

use anyhow::Result;
use axoupdater::AxoUpdater;
use clap::Args;

use joy_core::vcs::Vcs;
use joy_core::{init, store};

use crate::color;
use crate::commands::{ai, auth};

const PKG_NAME: &str = "joy";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

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
        return run_check();
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
        let summary = if changed == 0 {
            "Joy update complete -- nothing to refresh.".to_string()
        } else if changed == 1 {
            "Joy update complete -- 1 artefact refreshed.".to_string()
        } else {
            format!("Joy update complete -- {changed} artefacts refreshed.")
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

/// Run the full per-repo sync: lazy-activation (`.gitattributes` +
/// merge driver) + the work `joy auth update` does (SECURITY.md,
/// project.yaml schema) + the work `joy ai update` does (AI tool
/// instruction files) + stamp the version marker.
///
/// Best-effort: each step is fenced; a failure in one routine prints
/// a warning and the rest still run. Output is each routine's own
/// stdout, prefixed by section banners; a unified "refreshed: ..."
/// protocol is left for a follow-up polish pass under JOY-0163-6B.
/// Terse write-path orchestrator: call each refresh routine and let
/// it print joy-style rows ONLY for items it actually changed. Returns
/// the total number of artefacts refreshed across all subsystems so
/// the caller can summarise.
pub(crate) fn run_full_sync(root: &Path) -> u32 {
    let mut changed = 0u32;

    // init::onboard handles the joy-core-side artefacts (.gitattributes
    // block, merge driver git config, hooks, embedded defaults, version
    // marker). Today it does not report what it touched; for the terse
    // path we simply note whether it succeeded.
    if let Err(e) = init::onboard(root) {
        eprintln!("warning: onboard sync failed: {e}");
    }

    match auth::run_update_default() {
        Ok(n) => changed += n,
        Err(e) => eprintln!("warning: auth update skipped: {e}"),
    }

    match ai::run_update_default() {
        Ok(n) => changed += n,
        Err(e) => eprintln!("warning: ai update skipped: {e}"),
    }

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

    println!();
    println!("{}", color::section("Repo state"));
    match init::last_sync_version(&root) {
        Some(v) if v == CURRENT_VERSION => println!(
            "  {}version marker {}",
            color::check_mark(),
            color::inactive(&format!("({v})"))
        ),
        Some(v) => {
            stale = true;
            println!(
                "  {}version marker {}",
                color::warn_mark(),
                color::warning(&format!("stale (last {v}, current {CURRENT_VERSION})"))
            );
        }
        None => {
            stale = true;
            println!(
                "  {}version marker {}",
                color::warn_mark(),
                color::warning("never synced")
            );
        }
    }

    println!();
    println!("{}", color::section("Git extensions"));
    let row = |ok: bool, name: &str, detail: &str| {
        let mark = if ok {
            color::check_mark()
        } else {
            color::warn_mark()
        };
        let status = if ok {
            color::inactive(detail)
        } else {
            color::warning(detail)
        };
        println!("  {mark}{name:<24} {status}");
    };

    let block_ok = |path: &std::path::Path, marker: &str, entries: &[&str]| {
        std::fs::read_to_string(path)
            .map(|s| s.contains(marker) && entries.iter().all(|e| s.contains(e)))
            .unwrap_or(false)
    };

    let gitignore_entries: Vec<&str> = joy_core::init::GITIGNORE_BASE_ENTRIES
        .iter()
        .map(|(p, _)| *p)
        .collect();
    let gitignore_ok = block_ok(
        &root.join(".gitignore"),
        joy_core::init::GITIGNORE_BLOCK_START,
        &gitignore_entries,
    );
    if !gitignore_ok {
        stale = true;
    }
    row(
        gitignore_ok,
        ".gitignore block",
        if gitignore_ok {
            "managed block present"
        } else {
            "missing or out of date"
        },
    );

    let attrs_ok = block_ok(
        &root.join(".gitattributes"),
        joy_core::init::GITATTRIBUTES_BLOCK_START,
        joy_core::init::GITATTRIBUTES_BASE_ENTRIES,
    );
    if !attrs_ok {
        stale = true;
    }
    row(
        attrs_ok,
        ".gitattributes block",
        if attrs_ok {
            "managed block present"
        } else {
            "missing or out of date"
        },
    );

    let vcs = joy_core::vcs::default_vcs();
    let driver_ok = vcs
        .config_get(&root, joy_core::init::MERGE_DRIVER_NAME_KEY)
        .ok()
        .as_deref()
        == Some(joy_core::init::MERGE_DRIVER_NAME_VALUE)
        && vcs
            .config_get(&root, joy_core::init::MERGE_DRIVER_CMD_KEY)
            .ok()
            .as_deref()
            == Some(joy_core::init::MERGE_DRIVER_CMD_VALUE);
    if !driver_ok {
        stale = true;
    }
    row(
        driver_ok,
        "merge.joy-yaml.driver",
        if driver_ok {
            "registered"
        } else {
            "missing or out of date"
        },
    );

    let hooks_path_ok =
        vcs.config_get(&root, "core.hooksPath").ok().as_deref() == Some(".joy/hooks");
    if !hooks_path_ok {
        stale = true;
    }
    row(
        hooks_path_ok,
        "core.hooksPath",
        if hooks_path_ok {
            ".joy/hooks"
        } else {
            "not pointing at .joy/hooks"
        },
    );

    println!();
    println!("{}", color::section("Embedded files"));
    use joy_core::embedded::FileStatus;
    let embedded_groups: &[(&[joy_core::embedded::EmbeddedFile], &str)] = &[
        (joy_core::init::HOOK_FILES, "hook"),
        (joy_core::init::CONFIG_FILES, "config default"),
        (joy_core::init::PROJECT_FILES, "project default"),
    ];
    for (files, _label) in embedded_groups {
        match joy_core::embedded::diff_files(&root, files) {
            Ok(diffs) => {
                for (target, status) in diffs {
                    let ok = matches!(status, FileStatus::UpToDate);
                    if !ok {
                        stale = true;
                    }
                    let detail = match status {
                        FileStatus::UpToDate => "up to date",
                        FileStatus::Outdated => "stale (would be re-rendered)",
                        FileStatus::Missing => "missing (would be installed)",
                    };
                    row(ok, target, detail);
                }
            }
            Err(e) => {
                stale = true;
                println!("  {}error reading embedded files: {e}", color::warn_mark());
            }
        }
    }

    println!();
    match auth::run_check_default() {
        Ok(true) => stale = true,
        Ok(false) => {}
        Err(e) => println!("  {}error: {e}", color::warn_mark()),
    }

    println!();
    match ai::run_check_default() {
        Ok(true) => stale = true,
        Ok(false) => {}
        Err(e) => println!("  {}error: {e}", color::warn_mark()),
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
