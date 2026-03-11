// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::Utc;
use clap::Args;

use joy_core::items;
use joy_core::model::item::Comment;
use joy_core::store;

use crate::color;

#[derive(Args)]
#[command(after_help = "\
Examples:
  joy comment IT-0001 \"Looks good, merging now\"
  joy comment EP-0002 \"Blocked by external API changes\"")]
pub struct CommentArgs {
    /// Item ID (e.g. IT-0001)
    id: String,

    /// Comment text (required)
    text: Option<String>,
}

pub fn run(args: CommentArgs) -> Result<()> {
    let text = match args.text {
        Some(t) => t,
        None => anyhow::bail!("text is required: joy comment <ID> \"your comment\""),
    };

    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let mut item = items::load_item(&root, &args.id)?;

    let author = get_git_email()?;
    let comment = Comment {
        author,
        date: Utc::now(),
        text,
    };

    item.comments.push(comment);
    item.updated = Utc::now();
    items::update_item(&root, &item)?;

    println!("Added comment to {} {}", color::id(&item.id), item.title);

    Ok(())
}

fn get_git_email() -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["config", "user.email"])
        .output()
        .map_err(|_| anyhow::anyhow!("failed to run git config user.email"))?;

    if !output.status.success() {
        anyhow::bail!("git config user.email not set.");
    }

    let email = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if email.is_empty() {
        anyhow::bail!("git config user.email is empty.");
    }

    Ok(email)
}
