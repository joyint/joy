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
    /// Merge the target branch into this one, invoked by CI.
    Ci(CiArgs),
}

/// What the CI job runs on a pull request (JOY-02AC-53).
///
/// A forge merges with plain git, which runs no merge driver, so a Joy
/// project cannot be merged from the web interface. This does the merge
/// where joy IS installed, with Joy's own rules for Joy's own files, and
/// publishes the result. What the forge then does is a fast forward,
/// which cannot fail.
#[derive(Args)]
struct CiArgs {
    /// The branch to merge in. Without it, the one the CI names.
    #[arg(long)]
    target: Option<String>,
    /// The remote to fetch from and publish to.
    #[arg(long, default_value = "origin")]
    remote: String,
    /// Merge only, do not publish. For trying it out.
    #[arg(long)]
    no_push: bool,
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
        MergeCommand::Ci(a) => run_ci(a),
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
    let our_t = joy_core::vcs::commit_unix_time(&args.ours_rev);
    let their_t = joy_core::vcs::commit_unix_time(&args.theirs_rev);
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

/// The branch a pull request wants to land on, as the CI announces it.
/// GitHub and Gitea Actions set the first, GitLab the second.
fn target_from_ci() -> Option<String> {
    for key in ["GITHUB_BASE_REF", "CI_MERGE_REQUEST_TARGET_BRANCH_NAME"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim().to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// The address on the merge commit. The forge names the person whose
/// push started this; without one the machine says so plainly.
fn ci_email() -> String {
    for key in ["JOY_CI_EMAIL", "GIT_AUTHOR_EMAIL", "GITLAB_USER_EMAIL"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim().to_string();
            if !value.is_empty() {
                return value;
            }
        }
    }
    "joy-ci@localhost".to_string()
}

fn run_ci(args: CiArgs) -> Result<()> {
    let root = joy_core::store::find_project_root(&std::env::current_dir()?)
        .context("no Joy project here")?;
    let target = args
        .target
        .or_else(target_from_ci)
        .context("no target branch: pass --target, or let the CI name it")?;

    let auth = joy_core::vcs::forge::Auth::LocalAs(joy_core::host::process_host());
    let remote_ref = format!("refs/remotes/{}/{}", args.remote, target);
    // The checkout usually carries the target already (the CI clones with
    // full history). Fetching keeps it current; a forge that cannot be
    // reached only matters when the target is missing here too.
    let fetched =
        joy_core::vcs::forge::fetch_ref(&root, &auth, &format!("refs/heads/{target}"), &remote_ref);
    let have_remote_ref = joy_core::vcs::forge::rev_exists(&root, &remote_ref);
    let source = if have_remote_ref {
        remote_ref
    } else if joy_core::vcs::forge::rev_exists(&root, &target) {
        target.clone()
    } else {
        fetched.with_context(|| format!("fetch {target} from {}", args.remote))?;
        anyhow::bail!("{target} is not in this checkout and could not be fetched");
    };

    let outcome = joy_core::vcs::forge::merge_resolving_joy(
        &root,
        &source,
        &format!("Merge {target} into the pull request branch [no-item]"),
        "Joy CI",
        &ci_email(),
    )?;

    if !outcome.merged {
        println!("{target} is already in this branch, nothing to do");
        return Ok(());
    }
    for path in &outcome.resolved {
        println!("resolved by Joy's rules: {path}");
    }
    if args.no_push {
        println!("merged, not published (--no-push)");
        return Ok(());
    }
    joy_core::vcs::forge::push_branch(&root, &auth)
        .with_context(|| format!("publish the merge to {}", args.remote))?;
    println!("merged {target} in and published the result");
    Ok(())
}
