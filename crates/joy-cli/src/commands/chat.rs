// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! `joy chat` -- terminal inspection of the git-native chats in
//! `.joy/chats` (JOY-01F3). Authors resolve through MemberRef display
//! (no-raw-ID rule); list and show only, sending happens in the app.

use anyhow::Result;
use clap::{Args, Subcommand};

#[derive(Args)]
pub struct ChatArgs {
    #[command(subcommand)]
    command: ChatCommand,
}

#[derive(Subcommand)]
enum ChatCommand {
    /// List chats, newest first
    List,
    /// Show a chat's messages
    Show { id: String },
    /// Send a message to a chat (use `general` for the team-wide chat)
    Send { id: String, text: Vec<String> },
    /// Add a member to a chat (re-adds someone who left)
    Add { id: String, member: String },
    /// Leave a chat (posts a notice; you can be re-added)
    Leave { id: String },
    /// Delete a chat for yourself, or for everyone with --for-all
    Delete {
        id: String,
        /// Freeze the chat for every participant (creator action)
        #[arg(long)]
        for_all: bool,
    },
    /// Rename a chat
    Rename { id: String, title: Vec<String> },
}

fn acting_member(root: &std::path::Path) -> Result<joy_core::member_ref::MemberRef> {
    let email = joy_core::event_log::get_git_email()
        .map_err(|e| anyhow::anyhow!("cannot determine your identity: {e}"))?;
    let _ = root; // identity refinement (anonymous mode) happens in joy-core resolution
    Ok(joy_core::member_ref::MemberRef::new(email))
}

fn load_or_general(root: &std::path::Path, id: &str) -> Result<joy_core::model::chat::Chat> {
    if id == joy_core::chats::GENERAL_CHAT_ID {
        return Ok(joy_core::chats::ensure_general(root, chrono::Utc::now())?);
    }
    joy_core::chats::load_chat(root, id)?.ok_or_else(|| anyhow::anyhow!("no chat with id {id}"))
}

pub fn run(args: ChatArgs) -> Result<()> {
    let root = joy_core::store::find_project_root(&std::env::current_dir()?)
        .ok_or_else(|| anyhow::anyhow!("not inside a Joy project"))?;
    match args.command {
        ChatCommand::List => {
            let chats = joy_core::chats::load_chats(&root)?;
            if chats.is_empty() {
                println!("No chats.");
                return Ok(());
            }
            println!("{:<14} {:<22} {:<9} TITLE", "ID", "UPDATED", "MESSAGES");
            for c in chats {
                println!(
                    "{:<14} {:<22} {:<9} {}",
                    c.id,
                    c.updated.format("%Y-%m-%d %H:%M"),
                    c.messages.len(),
                    c.title.as_deref().unwrap_or("-"),
                );
            }
        }
        ChatCommand::Send { id, text } => {
            let me = acting_member(&root)?;
            let mut chat = load_or_general(&root, &id)?;
            let text = text.join(" ");
            if text.trim().is_empty() {
                anyhow::bail!("nothing to send");
            }
            joy_core::chats::append_message(&root, &mut chat, me, text, chrono::Utc::now())?;
            println!("sent to {}", chat.title.as_deref().unwrap_or(&chat.id));
        }
        ChatCommand::Add { id, member } => {
            let me = acting_member(&root)?;
            let mut chat = load_or_general(&root, &id)?;
            joy_core::chats::add_participant(
                &root,
                &mut chat,
                joy_core::member_ref::MemberRef::new(member),
                &me,
                chrono::Utc::now(),
            )?;
            println!("added");
        }
        ChatCommand::Leave { id } => {
            let me = acting_member(&root)?;
            let mut chat = load_or_general(&root, &id)?;
            joy_core::chats::leave(&root, &mut chat, &me, chrono::Utc::now())?;
            println!("left {}", chat.title.as_deref().unwrap_or(&chat.id));
        }
        ChatCommand::Delete { id, for_all } => {
            let me = acting_member(&root)?;
            let mut chat = load_or_general(&root, &id)?;
            if for_all {
                joy_core::chats::delete_for_all(&root, &mut chat, &me, chrono::Utc::now())?;
                println!("deleted for everyone (read-only until each member removes it)");
            } else {
                joy_core::chats::delete_for_me(&root, &mut chat, &me, chrono::Utc::now())?;
                println!("deleted for you");
            }
        }
        ChatCommand::Rename { id, title } => {
            let mut chat = load_or_general(&root, &id)?;
            joy_core::chats::rename(&root, &mut chat, title.join(" "))?;
            println!("renamed");
        }
        ChatCommand::Show { id } => {
            let chat = joy_core::chats::load_chat(&root, &id)?
                .ok_or_else(|| anyhow::anyhow!("no chat with id {id}"))?;
            println!(
                "{} — {} participant(s)",
                chat.title.as_deref().unwrap_or("(untitled)"),
                chat.participants.len()
            );
            for m in &chat.messages {
                println!(
                    "{}  {}  {}",
                    m.at.format("%Y-%m-%d %H:%M"),
                    m.author.id(),
                    m.text
                );
            }
        }
    }
    Ok(())
}
