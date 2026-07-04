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
