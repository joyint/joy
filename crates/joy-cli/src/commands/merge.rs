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
    /// Try the merge of the target branch into this one, invoked by CI.
    Ci(CiArgs),
}

/// What the CI job runs on a pull request (JOY-02AC-53, JOY-02DD-D2).
///
/// A forge merges with plain git, which runs no merge driver, so a pull
/// request whose Joy files meet the target's shows a conflict there. This
/// tries that merge where joy IS installed, with Joy's own rules for
/// Joy's own files, and publishes nothing: a push by the job's token puts
/// the pull request's checks behind a manual approval on GitHub since
/// 2026. Instead it writes the author a note, only when Joy files stood
/// in the way, saying what to do in their own clone, where `joy init`
/// registered the merge driver.
#[derive(Args)]
struct CiArgs {
    /// The branch to merge in. Without it, the one the CI names.
    #[arg(long)]
    target: Option<String>,
    /// The pull request's own branch, for the note. Without it, the one
    /// the CI names, else the checked out one.
    #[arg(long)]
    head: Option<String>,
    /// The remote to fetch the target from.
    #[arg(long, default_value = "origin")]
    remote: String,
    /// Where to write the note for the author. Written only when there is
    /// something to say; the CI posts it as a comment.
    #[arg(long)]
    advise: Option<PathBuf>,
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

/// The pull request's own branch, as the CI announces it. GitHub and
/// Gitea Actions set the first, GitLab the second.
fn head_from_ci() -> Option<String> {
    for key in ["GITHUB_HEAD_REF", "CI_MERGE_REQUEST_SOURCE_BRANCH_NAME"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim().to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// The note for the author when the forge cannot merge this pull request
/// by itself (JOY-02DD-D2). Three commands they can paste: switch to the
/// branch, pull the target, push. The pull runs the merge driver `joy
/// init` registered, so the Joy files resolve on their machine without a
/// word about drivers. `conflicts` are the files outside Joy's own that
/// both sides changed; they are theirs to settle in that same pull.
fn advice(head: &str, target: &str, remote: &str, conflicts: &[String]) -> String {
    let mut note = format!(
        "This branch and `{target}` both changed Joy files (items or logs). \
         The forge cannot merge those, joy on your machine can:\n\
         \n\
         ```\n\
         git switch {head}\n\
         git pull {remote} {target}\n\
         git push\n\
         ```\n\
         \n\
         joy resolves the Joy files during the pull, your code merges as usual."
    );
    if conflicts.is_empty() {
        note.push_str(" After the push the merge button turns green.\n");
    } else {
        note.push_str(&format!(
            " git also stops at {}: both sides changed it, so decide that in the \
             same pull, then commit and push.\n",
            conflicts
                .iter()
                .map(|path| format!("`{path}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    note
}

/// The files outside Joy's own that both sides changed, as
/// `merge_resolving_joy` names them when it stops.
fn conflicts_named(err: &anyhow::Error) -> Option<Vec<String>> {
    let text = err.to_string();
    let (_, list) = text.split_once("both sides changed the same thing in: ")?;
    Some(list.split(", ").map(str::to_string).collect())
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

    let head = args
        .head
        .or_else(head_from_ci)
        .or_else(|| joy_core::vcs::forge::head_branch(&root))
        .context("no head branch: pass --head, or let the CI name it")?;

    // The merge stays in this checkout and goes nowhere: it only tells
    // whether plain git, which is all the forge has, could do it alone.
    let note = match joy_core::vcs::forge::merge_resolving_joy(
        &root,
        &source,
        &format!("Merge {target} into the pull request branch [no-item]"),
        "Joy CI",
        &ci_email(),
    ) {
        Ok(outcome) if !outcome.merged => {
            println!("{target} is already in this branch, nothing to do");
            None
        }
        Ok(outcome) if outcome.resolved.is_empty() => {
            println!("plain git merges {target} in by itself, nothing to say");
            None
        }
        Ok(outcome) => {
            for path in &outcome.resolved {
                println!("only Joy's rules merge: {path}");
            }
            Some(advice(&head, &target, &args.remote, &[]))
        }
        Err(err) => match conflicts_named(&err) {
            Some(conflicts) => Some(advice(&head, &target, &args.remote, &conflicts)),
            None => return Err(err),
        },
    };

    let Some(note) = note else { return Ok(()) };
    print!("{note}");
    if let Some(path) = &args.advise {
        std::fs::write(path, &note)
            .with_context(|| format!("write the note to {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three commands the author pastes, and nothing about drivers
    /// (JOY-02DD-D2).
    #[test]
    fn the_note_names_the_branches_and_the_three_commands() {
        let note = advice("feat/passphrase-meter", "main", "origin", &[]);
        assert_eq!(
            note,
            "This branch and `main` both changed Joy files (items or logs). \
             The forge cannot merge those, joy on your machine can:\n\
             \n\
             ```\n\
             git switch feat/passphrase-meter\n\
             git pull origin main\n\
             git push\n\
             ```\n\
             \n\
             joy resolves the Joy files during the pull, your code merges as usual. \
             After the push the merge button turns green.\n"
        );
        assert!(!note.contains("driver"), "{note}");
    }

    /// A file outside Joy's own that both sides changed is the author's
    /// to settle, in the same pull.
    #[test]
    fn the_note_names_what_git_stops_at() {
        let note = advice(
            "fix/one",
            "develop",
            "origin",
            &["README.md".to_string(), "src/lib.rs".to_string()],
        );
        assert!(note.contains("git pull origin develop\n"), "{note}");
        assert!(
            note.ends_with(
                "git also stops at `README.md`, `src/lib.rs`: both sides changed it, \
                 so decide that in the same pull, then commit and push.\n"
            ),
            "{note}"
        );
        assert!(!note.contains("turns green"), "{note}");
    }

    /// What merge_resolving_joy says when it stops, read back as paths.
    #[test]
    fn the_stopped_merge_names_its_files() {
        let err = anyhow::anyhow!("both sides changed the same thing in: README.md, src/lib.rs");
        assert_eq!(
            conflicts_named(&err),
            Some(vec!["README.md".to_string(), "src/lib.rs".to_string()])
        );
        assert_eq!(conflicts_named(&anyhow::anyhow!("target: not found")), None);
    }
}
