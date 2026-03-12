// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::Utc;
use clap::Args;

use joy_core::items;
use joy_core::model::item::Comment;
use joy_core::store;
use joy_core::vcs::Vcs;

use crate::color;

#[derive(Args)]
#[command(after_help = "\
Examples:
  joy comment IT-0001 \"Looks good, merging now\"
  joy comment EP-0002 \"Blocked by external API changes\"")]
pub struct CommentArgs {
    /// Item ID (e.g. IT-0001)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
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

    let author = joy_core::vcs::default_vcs()
        .user_email()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let log_text = text.clone();
    let comment = Comment {
        author,
        date: Utc::now(),
        text,
    };

    item.comments.push(comment);
    item.updated = Utc::now();
    items::update_item(&root, &item)?;

    joy_core::event_log::log_event(
        &root,
        joy_core::event_log::EventType::CommentAdded,
        &item.id,
        Some(&log_text),
    );

    println!("Added comment to {} {}", color::id(&item.id), item.title);

    Ok(())
}
