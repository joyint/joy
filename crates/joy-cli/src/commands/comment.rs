// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::Utc;
use clap::{Args, Subcommand};

use joy_core::guard::Action;
use joy_core::items;
use joy_core::model::item::Comment;

use crate::color;

#[derive(Args)]
#[command(
    args_conflicts_with_subcommands = true,
    after_help = "\
Examples:
  joy comment IT-0001 \"Looks good, merging now\"
  joy comment IT-0001                       # opens $EDITOR
  joy comment edit IT-0001 2 \"fixed text\"   # replace comment #2
  joy comment rm   IT-0001 2                # delete comment #2

Comment indices are 1-based and match what `joy show <ID>` displays."
)]
pub struct CommentArgs {
    #[command(subcommand)]
    command: Option<CommentCommand>,

    /// Item ID for the bare form (e.g. IT-0001).
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    id: Option<String>,

    /// Comment text for the bare form. If omitted, an editor is opened.
    text: Option<String>,

    /// Editor command to use when TEXT is omitted (overrides config / $VISUAL / $EDITOR).
    #[arg(long)]
    editor: Option<String>,
}

#[derive(Subcommand)]
enum CommentCommand {
    /// Replace the text of an existing comment (1-based index)
    Edit(EditArgs),
    /// Remove a comment (1-based index)
    Rm(RmArgs),
}

#[derive(Args)]
struct EditArgs {
    /// Item ID
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    id: String,
    /// 1-based comment index as shown by `joy show`
    index: usize,
    /// New comment text. If omitted, the existing text is loaded into $EDITOR.
    text: Option<String>,
    /// Editor command to use when TEXT is omitted
    #[arg(long)]
    editor: Option<String>,
}

#[derive(Args)]
struct RmArgs {
    /// Item ID
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    id: String,
    /// 1-based comment index as shown by `joy show`
    index: usize,
    /// Skip the confirmation prompt
    #[arg(long)]
    force: bool,
}

pub fn run(args: CommentArgs) -> Result<()> {
    if let Some(cmd) = args.command {
        return match cmd {
            CommentCommand::Edit(a) => run_edit(a),
            CommentCommand::Rm(a) => run_rm(a),
        };
    }
    let id = args
        .id
        .ok_or_else(|| anyhow::anyhow!("usage: joy comment <ID> [TEXT]"))?;
    run_add(id, args.text, args.editor.as_deref())
}

fn run_add(id: String, text: Option<String>, editor: Option<&str>) -> Result<()> {
    let text = match text {
        Some(t) => t,
        None => match crate::editor::edit_text(editor, "", "comment.md")? {
            Some(t) => t,
            None => {
                println!("Empty comment, nothing added.");
                return Ok(());
            }
        },
    };

    let ctx = crate::crypt_session::load_context(None)?;
    let mut item = items::load_item(&ctx.root, &id)?;
    ctx.enforce(&Action::AddComment, &item.id)?;

    let comment = Comment {
        author: ctx.log_user(),
        date: Utc::now(),
        text,
    };
    item.comments.push(comment);
    item.updated = Utc::now();
    item.updated_by = Some(ctx.log_user());
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

fn run_edit(args: EditArgs) -> Result<()> {
    let ctx = crate::crypt_session::load_context(None)?;
    let mut item = items::load_item(&ctx.root, &args.id)?;
    ctx.enforce(&Action::AddComment, &item.id)?;

    let pos = resolve_index(&item.comments, args.index)?;
    let old_text = item.comments[pos].text.clone();
    let new_text = match args.text {
        Some(t) => t,
        None => match crate::editor::edit_text(args.editor.as_deref(), &old_text, "comment.md")? {
            Some(t) => t,
            None => {
                println!("Empty comment text, edit aborted.");
                return Ok(());
            }
        },
    };

    item.comments[pos].text = new_text;
    item.comments[pos].author = ctx.log_user();
    item.comments[pos].date = Utc::now();
    item.updated = Utc::now();
    item.updated_by = Some(ctx.log_user());
    items::update_item(&ctx.root, &item)?;

    joy_core::event_log::log_event_as(
        &ctx.root,
        joy_core::event_log::EventType::CommentEdited,
        &item.id,
        Some(&format!("[{}]", args.index)),
        &ctx.log_user(),
    );

    if crate::output::is_json() {
        crate::output::emit(&item)?;
    } else {
        println!(
            "Edited comment #{} on {} {}",
            args.index,
            color::id(&item.id),
            item.title
        );
    }
    joy_core::git_ops::auto_git_post_command(
        &ctx.root,
        &format!("comment edit {} {}", item.id, args.index),
        &ctx.log_user(),
    );
    Ok(())
}

fn run_rm(args: RmArgs) -> Result<()> {
    let ctx = crate::crypt_session::load_context(None)?;
    let mut item = items::load_item(&ctx.root, &args.id)?;
    ctx.enforce(&Action::AddComment, &item.id)?;

    let pos = resolve_index(&item.comments, args.index)?;
    let preview = preview_text(&item.comments[pos].text);

    if !args.force
        && !crate::output::is_json()
        && !crate::prompt::ask_yn(
            &format!(
                "Remove comment #{} ({}) on {}?",
                args.index, preview, item.id
            ),
            false,
        )?
    {
        println!("Aborted.");
        return Ok(());
    }

    item.comments.remove(pos);
    item.updated = Utc::now();
    item.updated_by = Some(ctx.log_user());
    items::update_item(&ctx.root, &item)?;

    joy_core::event_log::log_event_as(
        &ctx.root,
        joy_core::event_log::EventType::CommentRemoved,
        &item.id,
        Some(&format!("[{}]", args.index)),
        &ctx.log_user(),
    );

    if crate::output::is_json() {
        crate::output::emit(&item)?;
    } else {
        println!(
            "Removed comment #{} on {} {}",
            args.index,
            color::id(&item.id),
            item.title
        );
    }
    joy_core::git_ops::auto_git_post_command(
        &ctx.root,
        &format!("comment rm {} {}", item.id, args.index),
        &ctx.log_user(),
    );
    Ok(())
}

fn resolve_index(comments: &[Comment], one_based: usize) -> Result<usize> {
    if one_based == 0 {
        anyhow::bail!("comment indices are 1-based");
    }
    let pos = one_based - 1;
    if pos >= comments.len() {
        anyhow::bail!(
            "no comment #{one_based}: item has {} comment(s)",
            comments.len()
        );
    }
    Ok(pos)
}

fn preview_text(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or("");
    if first_line.chars().count() <= 60 {
        first_line.to_string()
    } else {
        let truncated: String = first_line.chars().take(57).collect();
        format!("{truncated}...")
    }
}
