// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::Utc;
use clap::Args;

use joy_core::guard::Action;
use joy_core::items;
use joy_core::model::item::Comment;

use crate::color;

#[derive(Args)]
#[command(after_help = "\
Examples:
  joy comment IT-0001 \"Looks good, merging now\"
  joy comment IT-0001                    # opens $EDITOR
  joy comment EP-0002 \"Blocked by external API changes\"")]
pub struct CommentArgs {
    /// Item ID (e.g. IT-0001)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    id: String,

    /// Comment text. If omitted, an editor is opened.
    text: Option<String>,

    /// Editor command to use when TEXT is omitted (overrides config / $VISUAL / $EDITOR).
    #[arg(long)]
    editor: Option<String>,
}

pub fn run(args: CommentArgs) -> Result<()> {
    let text = match args.text {
        Some(t) => t,
        None => match crate::editor::edit_text(args.editor.as_deref(), "", "comment.md")? {
            Some(t) => t,
            None => {
                println!("Empty comment, nothing added.");
                return Ok(());
            }
        },
    };

    let ctx = crate::crypt_session::load_context(None)?;

    let mut item = items::load_item(&ctx.root, &args.id)?;

    ctx.enforce(&Action::AddComment, &item.id)?;

    let comment = Comment {
        author: ctx.log_user(),
        date: Utc::now(),
        text,
    };

    item.comments.push(comment);
    item.updated = Utc::now();
    items::update_item(&ctx.root, &item)?;

    joy_core::event_log::log_event_as(
        &ctx.root,
        joy_core::event_log::EventType::CommentAdded,
        &item.id,
        None,
        &ctx.log_user(),
    );

    if crate::output::is_json() {
        crate::output::emit(&item)?;
    } else {
        println!("Added comment to {} {}", color::id(&item.id), item.title);
    }

    joy_core::git_ops::auto_git_post_command(
        &ctx.root,
        &format!("comment {} {}", item.id, item.title),
        &ctx.log_user(),
    );

    Ok(())
}
