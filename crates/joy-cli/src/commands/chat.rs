// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! `joy chat` -- terminal inspection of the git-native chats in
//! `.joy/chats` (JOY-01F3). Authors resolve through MemberRef display
//! (no-raw-ID rule); list and show only, sending happens in the app.

use anyhow::Result;
use clap::{Args, Subcommand};

use joy_core::vcs::Vcs;

#[derive(Args)]
pub struct ChatArgs {
    #[command(subcommand)]
    command: ChatCommand,

    /// Passphrase of the acting member, to read/write sealed chats
    /// (non-interactive). Without it the CLI only sees unsealed chats.
    #[arg(long, global = true)]
    passphrase: Option<String>,

    /// Read the passphrase from a single line on stdin.
    #[arg(long = "passphrase-stdin", global = true)]
    passphrase_stdin: bool,
}

#[derive(Subcommand)]
enum ChatCommand {
    /// List chats, newest first
    List {
        /// Also list chats you left or deleted that still exist.
        #[arg(long, short)]
        all: bool,
    },
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
    /// Mark a chat read up to now (advance your read marker)
    Read { id: String },
    /// Show read state: who has read the latest message and unread counts
    Info { id: String },
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

/// Reading or writing a sealed chat needs the caller's identity seed. When
/// a passphrase is supplied (and the acting member has an identity), unlock
/// it and set it as the reader/writer seed. Without a passphrase we stay on
/// the legacy/plaintext path, so a bare `joy init` project (no identity)
/// keeps working with no prompt.
fn establish_reader_seed(
    root: &std::path::Path,
    passphrase: Option<&str>,
    stdin: bool,
) -> Result<()> {
    if passphrase.is_none() && !stdin {
        return Ok(());
    }
    let Ok(project) = joy_core::store::load_project(root) else {
        return Ok(());
    };
    let Ok(email) = joy_core::vcs::default_vcs().user_email() else {
        return Ok(());
    };
    let Some(member) = project.member_by_email(&email) else {
        return Ok(());
    };
    if member.verify_key.is_none() {
        return Ok(());
    }
    let pass = crate::commands::auth::read_passphrase(passphrase, stdin, "Passphrase: ")?;
    let unlocked = joy_core::auth::unlock_identity(member, &pass)?;
    joy_core::chat_crypt::set_custodian_seed(Some(unlocked.seed));
    Ok(())
}

pub fn run(args: ChatArgs) -> Result<()> {
    let root = joy_core::store::find_project_root(&std::env::current_dir()?)
        .ok_or_else(|| anyhow::anyhow!("not inside a Joy project"))?;
    establish_reader_seed(&root, args.passphrase.as_deref(), args.passphrase_stdin)?;
    match args.command {
        ChatCommand::List { all } => {
            let me = acting_member(&root).ok();
            let chats = joy_core::chats::load_chats(&root)?;
            // Default: only chats you are a member of and have not deleted.
            // `--all` also shows chats you left or deleted that still exist.
            let rows: Vec<_> = chats
                .into_iter()
                .filter(|c| {
                    all || me
                        .as_ref()
                        .map(|me| joy_core::chats::visible_to(c, me))
                        .unwrap_or(true)
                })
                .collect();
            if rows.is_empty() {
                println!("No chats.");
                return Ok(());
            }
            println!("{:<14} {:<22} {:<9} TITLE", "ID", "UPDATED", "MESSAGES");
            for c in rows {
                let left = me
                    .as_ref()
                    .map(|me| !joy_core::chats::visible_to(&c, me))
                    .unwrap_or(false);
                let title = c.title.as_deref().unwrap_or("-");
                println!(
                    "{:<14} {:<22} {:<9} {}{}",
                    c.id,
                    c.updated.format("%Y-%m-%d %H:%M"),
                    c.messages.len(),
                    title,
                    if left { " [left]" } else { "" },
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
        ChatCommand::Read { id } => {
            let me = acting_member(&root)?;
            let mut chat = load_or_general(&root, &id)?;
            joy_core::chats::mark_read(&root, &mut chat, &me, chrono::Utc::now())?;
            println!("marked read");
        }
        ChatCommand::Info { id } => {
            let me = acting_member(&root)?;
            let chat = load_or_general(&root, &id)?;
            println!(
                "{} — {} participant(s), {} message(s)",
                chat.title.as_deref().unwrap_or("(untitled)"),
                chat.participants.len(),
                chat.messages.len(),
            );
            println!("your unread: {}", chat.unread_count(me.id()));
            if let Some(last) = chat.messages.last() {
                let readers = chat.read_by(last);
                println!(
                    "latest read by {}/{}: {}",
                    readers.len(),
                    chat.participants.len(),
                    if readers.is_empty() {
                        "(nobody yet)".to_string()
                    } else {
                        readers.join(", ")
                    },
                );
            }
            for p in &chat.participants {
                println!("  {:<28} {} unread", p.id(), chat.unread_count(p.id()));
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
