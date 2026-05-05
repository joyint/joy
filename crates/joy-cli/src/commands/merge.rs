// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Hidden git merge driver entry point. Invoked by Git through the
//! `.gitattributes` block written by `joy init` / `joy onboard`. End
//! users do not run this directly; the subcommand is hidden from help.
//!
//! For plaintext YAML files the driver delegates to
//! [`joy_core::merge::merge_yaml_doc`], which always resolves to a
//! valid document.
//!
//! For Joy-encrypted blobs (JOYCRYPT magic) we never attempt a
//! field-level merge: the content is opaque ciphertext. Instead we
//! pick the side whose revision has the later commit timestamp (Git's
//! `%X` / `%Y` revision name placeholders, resolved via
//! `git log -1 --format=%ct`). When that information is unavailable we
//! default to the incoming side (`theirs`).

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};

use joy_core::merge;

#[derive(Args)]
#[command(hide = true, about = "Internal: Git merge driver helpers")]
pub struct MergeArgs {
    #[command(subcommand)]
    command: MergeCommand,
}

#[derive(Subcommand)]
enum MergeCommand {
    /// Three-way YAML merge driver invoked by Git.
    Driver(DriverArgs),
}

#[derive(Args)]
struct DriverArgs {
    /// Common ancestor (Git's %O placeholder).
    #[arg(long)]
    base: PathBuf,
    /// Current side, will be overwritten with the merge result (Git's %A).
    #[arg(long)]
    current: PathBuf,
    /// Other side (Git's %B).
    #[arg(long)]
    other: PathBuf,
    /// Path of the file in the working tree, for diagnostics (Git's %P).
    #[arg(long, default_value = "")]
    path: String,
    /// Current side's revision name (Git's %X).
    #[arg(long, default_value = "")]
    ours_rev: String,
    /// Other side's revision name (Git's %Y).
    #[arg(long, default_value = "")]
    theirs_rev: String,
}

pub fn run(args: MergeArgs) -> Result<()> {
    match args.command {
        MergeCommand::Driver(a) => run_driver(a),
    }
}

fn run_driver(args: DriverArgs) -> Result<()> {
    let base_bytes = std::fs::read(&args.base)
        .with_context(|| format!("read base file {}", args.base.display()))?;
    let ours_bytes = std::fs::read(&args.current)
        .with_context(|| format!("read current file {}", args.current.display()))?;
    let theirs_bytes = std::fs::read(&args.other)
        .with_context(|| format!("read other file {}", args.other.display()))?;

    let any_encrypted = merge::is_joycrypt_blob(&base_bytes)
        || merge::is_joycrypt_blob(&ours_bytes)
        || merge::is_joycrypt_blob(&theirs_bytes);

    if any_encrypted {
        let chosen = pick_newer_encrypted(&args, ours_bytes, theirs_bytes);
        std::fs::write(&args.current, chosen)
            .with_context(|| format!("write merged result to {}", args.current.display()))?;
        return Ok(());
    }

    let base = String::from_utf8(base_bytes)
        .with_context(|| format!("base {} is not valid UTF-8", args.base.display()))?;
    let ours = String::from_utf8(ours_bytes)
        .with_context(|| format!("current {} is not valid UTF-8", args.current.display()))?;
    let theirs = String::from_utf8(theirs_bytes)
        .with_context(|| format!("other {} is not valid UTF-8", args.other.display()))?;

    let merged = merge::merge_yaml_doc(&base, &ours, &theirs).with_context(|| {
        format!(
            "merge {}",
            if args.path.is_empty() {
                args.current.display().to_string()
            } else {
                args.path.clone()
            }
        )
    })?;

    std::fs::write(&args.current, merged)
        .with_context(|| format!("write merged result to {}", args.current.display()))?;

    Ok(())
}

fn pick_newer_encrypted(args: &DriverArgs, ours: Vec<u8>, theirs: Vec<u8>) -> Vec<u8> {
    let our_t = commit_unix_time(&args.ours_rev);
    let their_t = commit_unix_time(&args.theirs_rev);
    let prefer_theirs = match (our_t, their_t) {
        (Some(o), Some(t)) => t >= o,
        _ => true,
    };
    if prefer_theirs {
        theirs
    } else {
        ours
    }
}

fn commit_unix_time(rev: &str) -> Option<i64> {
    let rev = rev.trim();
    if rev.is_empty() || rev.starts_with('%') {
        return None;
    }
    let out = Command::new("git")
        .args(["log", "-1", "--format=%ct", rev])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}
