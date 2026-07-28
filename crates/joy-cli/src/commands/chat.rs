// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! `joy chat` -- the git-native chats on `refs/joy/chats` in the
//! terminal (JOY-01F3, JOY-0227-5E). Authors resolve through MemberRef
//! display (no-raw-ID rule). Every verb syncs the chat ref itself
//! (ADR JAPP-00DC-FC): reads fetch first so platform/app replies are
//! visible, writes persist locally first (truth in the repo) and then
//! fetch/merge/push in the same invocation -- no manual
//! `git push refs/joy/...` ever.

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
    #[command(alias = "list")]
    Ls {
        /// Also list chats you left or deleted that still exist.
        #[arg(long, short)]
        all: bool,
        /// Only chats with an @mention at me; LAST/UNREAD scope to those
        /// mentions (my mention inbox).
        #[arg(long)]
        mine: bool,
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

/// Sync `refs/joy/chats` with the project's remote (JOY-0227-5E): fetch
/// into the tracking ref, reconcile locally through joy-chat (adopt /
/// fast-forward / message-union merge), push when the local ref is
/// ahead. Network runs through the git CLI so the user's ambient auth
/// (ssh agent, credential helper) applies -- no token plumbing.
///
/// NEVER fatal: a chat is committed locally before any network I/O, so a
/// failed sync only delays visibility. Failures classify like the app
/// sync worker: a missing remote ref is the normal first sync; auth
/// errors are permanent (fix access); everything else is transient and
/// the next send or read retries.
fn sync_ref(root: &std::path::Path) {
    let remote = joy_core::store::load_config()
        .sync
        .map(|s| s.remote)
        .unwrap_or_else(|| "origin".to_string());
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
    };
    // A project without this remote is local-only: chats stay local.
    match git(&["remote", "get-url", &remote]) {
        Ok(out) if out.status.success() => {}
        _ => return,
    }
    let fetch_spec = format!(
        "+{}:{}",
        joy_chat_store::chat_ref::CHATS_REF,
        joy_chat_store::chat_ref::CHATS_TRACKING_REF
    );
    match git(&["fetch", "--quiet", &remote, &fetch_spec]) {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            // No remote chats yet: the normal first sync, nothing to merge.
            if !stderr.contains("couldn't find remote ref") {
                eprintln!(
                    "chats not fetched ({}); local state shown, the next send or read retries",
                    classify_sync_error(&stderr)
                );
            }
        }
        Err(e) => {
            eprintln!("chats not fetched (git unavailable: {e}); local state shown");
            return;
        }
    }
    let push_needed = match joy_chat_store::chat_ref::reconcile_with_tracking(root) {
        Ok(needed) => needed,
        Err(e) => {
            eprintln!("chat ref reconcile failed: {e}");
            return;
        }
    };
    if !push_needed {
        return;
    }
    let push_spec = format!(
        "{}:{}",
        joy_chat_store::chat_ref::CHATS_REF,
        joy_chat_store::chat_ref::CHATS_REF
    );
    match git(&["push", "--quiet", &remote, &push_spec]) {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            eprintln!(
                "chat stays committed locally, push failed ({}); the next send or read retries",
                classify_sync_error(&stderr)
            );
        }
        Err(e) => eprintln!("chat stays committed locally, push failed (git unavailable: {e})"),
    }
}

/// One-line failure classification, mirroring the app sync worker's
/// transient/permanent split.
fn classify_sync_error(stderr: &str) -> String {
    let s = stderr.to_lowercase();
    if s.contains("permission denied")
        || s.contains("authentication")
        || s.contains("403")
        || s.contains("401")
        || s.contains("access denied")
    {
        "no access to the remote -- check your credentials".to_string()
    } else if s.contains("could not resolve")
        || s.contains("unable to access")
        || s.contains("connection")
    {
        "offline?".to_string()
    } else {
        let line = stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        format!("transient: {}", line.trim())
    }
}

/// The @mention view of one chat for `me` (JOY-0226-27): when the newest
/// mention happened, computed once per row.
struct MentionInbox {
    last: Option<chrono::DateTime<chrono::Utc>>,
}

impl MentionInbox {
    /// Unread mentions at me: mention messages strictly after my
    /// effective read watermark.
    fn unread_mentions(&self, chat: &joy_chat::model::chat::Chat, me: &str) -> usize {
        let watermark = chat.effective_watermark(me);
        let me_key = [me.to_string()];
        chat.messages
            .iter()
            .filter(|m| !joy_chat::mentions::mentions(&m.text, &me_key).is_empty())
            .filter(|m| watermark.is_none_or(|w| m.at > w))
            .count()
    }
}

fn mention_inbox(chat: &joy_chat::model::chat::Chat, me: &str) -> MentionInbox {
    let me_key = [me.to_string()];
    let last = chat
        .messages
        .iter()
        .filter(|m| !joy_chat::mentions::mentions(&m.text, &me_key).is_empty())
        .map(|m| m.at)
        .max();
    MentionInbox { last }
}

fn load_or_general(root: &std::path::Path, id: &str) -> Result<joy_chat::model::chat::Chat> {
    if id == joy_chat_store::chats::GENERAL_CHAT_ID {
        return Ok(joy_chat_store::chats::ensure_general(
            root,
            chrono::Utc::now(),
        )?);
    }
    joy_chat_store::chats::load_chat(root, id)?
        .ok_or_else(|| anyhow::anyhow!("no chat with id {id}"))
}

/// The chat key this SESSION carries, if it carries one.
///
/// An AI acts with a token and never with a passphrase: under the crypt
/// scope its delegation private key rides in the session (ADR-041 §5),
/// and that key IS its identity for chats, exactly as an unwrapped seed
/// is a person's. Zone keys already work this way; chats did not, so an
/// AI on the command line saw "No chats" in rooms it is a member of
/// (JOY-023E-68). `--session` is copied into the environment before any
/// command runs, so reading it here covers both ways of passing one.
fn session_chat_seed() -> Option<[u8; 32]> {
    let env_value = std::env::var("JOY_SESSION").ok()?;
    let (_sid, _ephemeral, delegation) =
        joy_core::auth::session::parse_session_env_full(&env_value)?;
    delegation
}

/// Reading or writing a sealed chat needs the caller's identity seed.
///
/// Where it comes from depends on who is acting, and on nothing else: a
/// session that carries one brings its own, a person unwraps theirs with
/// their passphrase. A person at a terminal who has an identity but gave
/// no passphrase is ASKED, rather than shown an empty room; a script
/// without one stays on the quiet path, so a bare `joy init` project (no
/// identity at all) keeps working with no prompt.
fn establish_reader_seed(
    root: &std::path::Path,
    passphrase: Option<&str>,
    stdin: bool,
) -> Result<()> {
    if let Some(seed) = session_chat_seed() {
        joy_chat_store::writer::set_seed(Some(seed));
        return Ok(());
    }
    // An AI has no other way in: no passphrase to type, and its session
    // is the only place its key could come from. Saying "no chats" here
    // would be a lie about the room; say what is actually missing.
    if let Ok(email) = joy_core::vcs::default_vcs().user_email() {
        if joy_core::model::project::is_ai_member(&email) {
            anyhow::bail!(
                "this session carries no chat key, so {email} cannot open the chats it is in. \
                 The delegation was issued auth-only; reissue it with the crypt scope \
                 (`joy auth token add {email} --crypt`) and redeem it again."
            );
        }
    }
    if passphrase.is_none() && !stdin && !crate::prompt::is_interactive() {
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
    joy_chat_store::writer::set_seed(Some(unlocked.seed));
    Ok(())
}

pub fn run(args: ChatArgs) -> Result<()> {
    let root = joy_core::store::find_project_root(&std::env::current_dir()?)
        .ok_or_else(|| anyhow::anyhow!("not inside a Joy project"))?;
    establish_reader_seed(&root, args.passphrase.as_deref(), args.passphrase_stdin)?;
    // EVERY verb pulls first (JOY-022A-4D): reads see replies from the
    // platform and other members, and writes append on the ADOPTED
    // remote state — sealing under the chat's existing crypt epoch. A
    // write on an unmerged local ref would build the chat from scratch
    // and mint a parallel epoch other holders (notably the platform
    // custodian) have no slot for. Writes sync again after the persist
    // to push the appended ref (JOY-0227-5E).
    let is_read = matches!(
        args.command,
        ChatCommand::Ls { .. } | ChatCommand::Show { .. } | ChatCommand::Info { .. }
    );
    sync_ref(&root);
    let result = run_command(&root, args.command);
    if result.is_ok() && !is_read {
        sync_ref(&root);
    }
    result
}

fn run_command(root: &std::path::Path, command: ChatCommand) -> Result<()> {
    match command {
        ChatCommand::Ls { all, mine } => {
            let me = acting_member(root).ok();
            if mine && me.is_none() {
                anyhow::bail!("--mine needs an identity (run joy auth init or pass a passphrase)");
            }
            let chats = joy_chat_store::chats::load_chats(root)?;
            // Default: only chats you are a member of and have not deleted.
            // `--all` also shows chats you left or deleted that still exist.
            let rows: Vec<_> = chats
                .into_iter()
                .filter(|c| {
                    all || me
                        .as_ref()
                        .map(|me| joy_chat_store::chats::visible_to(c, me))
                        .unwrap_or(true)
                })
                .filter_map(|c| {
                    let inbox = me.as_ref().map(|me| mention_inbox(&c, me.id()));
                    if mine && inbox.as_ref().is_none_or(|i| i.last.is_none()) {
                        return None;
                    }
                    Some((c, inbox))
                })
                .collect();
            if rows.is_empty() {
                println!("{}", if mine { "No mentions." } else { "No chats." });
                return Ok(());
            }
            println!(
                "{:<14} {:<17} {:<5} {:<17} {:<7} TITLE",
                "ID", "UPDATED", "MSGS", "LAST@ME", "UNREAD"
            );
            for (c, inbox) in rows {
                let left = me
                    .as_ref()
                    .map(|me| !joy_chat_store::chats::visible_to(&c, me))
                    .unwrap_or(false);
                let title = c.title.as_deref().unwrap_or("-");
                // UNREAD: with --mine only the unread mentions at me, else
                // every message after my effective watermark (JOY-0226-27).
                let unread = match (&me, &inbox) {
                    (Some(me), Some(inbox)) if mine => inbox.unread_mentions(&c, me.id()),
                    (Some(me), _) => c.unread_count(me.id()),
                    (None, _) => 0,
                };
                let last = inbox
                    .as_ref()
                    .and_then(|i| i.last)
                    .map(|at| at.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "-".into());
                println!(
                    "{:<14} {:<17} {:<5} {:<17} {:<7} {}{}",
                    c.id,
                    c.updated.format("%Y-%m-%d %H:%M"),
                    c.messages.len(),
                    last,
                    if me.is_some() {
                        unread.to_string()
                    } else {
                        "-".into()
                    },
                    title,
                    if left { " [left]" } else { "" },
                );
            }
        }
        ChatCommand::Send { id, text } => {
            let me = acting_member(root)?;
            let mut chat = load_or_general(root, &id)?;
            let text = text.join(" ");
            if text.trim().is_empty() {
                anyhow::bail!("nothing to send");
            }
            joy_chat_store::chats::append_message(root, &mut chat, me, text, chrono::Utc::now())?;
            println!("sent to {}", chat.title.as_deref().unwrap_or(&chat.id));
        }
        ChatCommand::Add { id, member } => {
            let me = acting_member(root)?;
            let mut chat = load_or_general(root, &id)?;
            joy_chat_store::chats::add_participant(
                root,
                &mut chat,
                joy_core::member_ref::MemberRef::new(member),
                &me,
                chrono::Utc::now(),
            )?;
            println!("added");
        }
        ChatCommand::Leave { id } => {
            let me = acting_member(root)?;
            let mut chat = load_or_general(root, &id)?;
            joy_chat_store::chats::leave(root, &mut chat, &me, chrono::Utc::now())?;
            println!("left {}", chat.title.as_deref().unwrap_or(&chat.id));
        }
        ChatCommand::Delete { id, for_all } => {
            let me = acting_member(root)?;
            let mut chat = load_or_general(root, &id)?;
            if for_all {
                joy_chat_store::chats::delete_for_all(root, &mut chat, &me, chrono::Utc::now())?;
                println!("deleted for everyone (read-only until each member removes it)");
            } else {
                joy_chat_store::chats::delete_for_me(root, &mut chat, &me, chrono::Utc::now())?;
                println!("deleted for you");
            }
        }
        ChatCommand::Rename { id, title } => {
            let mut chat = load_or_general(root, &id)?;
            joy_chat_store::chats::rename(root, &mut chat, title.join(" "))?;
            println!("renamed");
        }
        ChatCommand::Read { id } => {
            let me = acting_member(root)?;
            let mut chat = load_or_general(root, &id)?;
            joy_chat_store::chats::mark_read(root, &mut chat, &me, chrono::Utc::now())?;
            println!("marked read");
        }
        ChatCommand::Info { id } => {
            let me = acting_member(root)?;
            let chat = load_or_general(root, &id)?;
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
            let chat = joy_chat_store::chats::load_chat(root, &id)?
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
