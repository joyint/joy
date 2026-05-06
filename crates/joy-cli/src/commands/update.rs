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

use anyhow::Result;
use axoupdater::AxoUpdater;
use clap::Args;

use joy_core::{init, store};

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

    let mut binary_msg: Option<String> = None;
    if !args.no_binary {
        binary_msg = Some(swap_binary());
    }

    if let Some(msg) = binary_msg {
        println!("{}", msg);
    }

    if let Some(root) = project_root {
        match init::run_sync(&root, CURRENT_VERSION) {
            Ok(()) => println!(
                "Synced this repo to joy {} (registered: .gitattributes, merge driver).",
                CURRENT_VERSION
            ),
            Err(e) => eprintln!("warning: in-repo sync failed: {e}"),
        }
    } else {
        println!("Not inside a Joy project; skipping in-repo sync.");
    }

    Ok(())
}

fn run_check() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let mut stale = false;

    // Binary version: ask axoupdater whether a newer release exists.
    let mut updater = AxoUpdater::new_for(PKG_NAME);
    if updater.load_receipt().is_err() {
        println!("binary: install receipt missing (managed by another installer)");
    } else {
        let upgrade = updater.is_update_needed_sync().unwrap_or(false);
        if upgrade {
            stale = true;
            println!("binary: update available (current {CURRENT_VERSION})");
        } else {
            println!("binary: up to date ({CURRENT_VERSION})");
        }
    }

    if let Some(root) = store::find_project_root(&cwd) {
        match init::last_sync_version(&root) {
            Some(v) if v == CURRENT_VERSION => {
                println!("repo: synced ({v})");
            }
            Some(v) => {
                stale = true;
                println!("repo: stale (last synced at {v}, current {CURRENT_VERSION})");
            }
            None => {
                stale = true;
                println!("repo: never synced (current {CURRENT_VERSION})");
            }
        }
    } else {
        println!("repo: not inside a Joy project");
    }

    if stale {
        std::process::exit(2);
    }
    Ok(())
}

fn swap_binary() -> String {
    let mut updater = AxoUpdater::new_for(PKG_NAME);
    if updater.load_receipt().is_err() {
        return format!(
            "Binary update skipped: no cargo-dist install receipt found at {}.\n\
             Update via the installer that placed this binary, e.g.\n  \
             cargo install --force joy-cli\n  brew upgrade joy",
            std::env::current_exe()
                .ok()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "<unknown path>".into())
        );
    }
    match updater.run_sync() {
        Ok(Some(result)) => format!(
            "Binary updated: {:?} -> {:?}",
            result.old_version, result.new_version
        ),
        Ok(None) => format!("Binary already up to date ({CURRENT_VERSION})."),
        Err(e) => format!("Binary update failed: {e}"),
    }
}
