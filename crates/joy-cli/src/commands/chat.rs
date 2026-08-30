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
    /// Show a chat's messages and mark it read (opening a chat is
    /// reading it, as in the apps); filters narrow what is shown
    Show {
        id: String,
        /// Only messages you have not read yet
        #[arg(long)]
        unread: bool,
        /// Only the last N messages
        #[arg(long, value_name = "N")]
        last: Option<usize>,
        /// Only messages newer than this: 30m, 2h, 3d
        #[arg(long, value_name = "AGE")]
        since: Option<String>,
    },
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
    /// Show read state: who has read the latest message and unread counts
    Info { id: String },
}

/// Who acts: the ONE identity resolution of joy-core, the same every
/// command uses (JOY-026E-0F). With a session (`--session`, JOY_SESSION)
/// that is the session's member, an AI delegated by a person; without
/// one it is the person at the terminal (git e-mail). Reading the git
/// e-mail directly here made a delegated AI post as the machine's owner.
fn acting_identity(root: &std::path::Path) -> Result<joy_core::identity::Identity> {
    joy_core::identity::resolve_identity(root)
        .map_err(|e| anyhow::anyhow!("cannot determine your identity: {e}"))
}

fn acting_member(root: &std::path::Path) -> Result<joy_core::member_ref::MemberRef> {
    Ok(acting_identity(root)?.member)
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
/// Push-first delivery after a write (JOY-026C-34): the local chats ref
/// goes up as it is; a refusal (someone else pushed first) is the one
/// case that fetches, unites and pushes again through [`sync_ref`].
fn deliver_ref(root: &std::path::Path) {
    let remote = joy_core::store::load_config()
        .sync
        .map(|s| s.remote)
        .unwrap_or_else(|| "origin".to_string());
    if !joy_core::vcs::remote_exists(root, &remote) {
        return;
    }
    let push_spec = format!(
        "{}:{}",
        joy_chat_store::chat_ref::CHATS_REF,
        joy_chat_store::chat_ref::CHATS_REF
    );
    match joy_core::vcs::push_ref(root, &remote, &push_spec) {
        joy_core::vcs::RefTransfer::Done => {}
        joy_core::vcs::RefTransfer::Refused(stderr) => {
            let lower = stderr.to_ascii_lowercase();
            // nothing local to push yet: not a refusal
            if lower.contains("src refspec") {
                return;
            }
            // the forge moved first: the full round unites and retries
            if lower.contains("rejected")
                || lower.contains("fetch first")
                || lower.contains("non-fast-forward")
            {
                sync_ref(root);
                return;
            }
            eprintln!(
                "chat stays committed locally, push failed ({}); the next send or read retries",
                classify_sync_error(&stderr)
            );
        }
        joy_core::vcs::RefTransfer::GitUnavailable(e) => {
            eprintln!("chat stays committed locally, push failed (git unavailable: {e})");
        }
    }
}

fn sync_ref(root: &std::path::Path) {
    let remote = joy_core::store::load_config()
        .sync
        .map(|s| s.remote)
        .unwrap_or_else(|| "origin".to_string());
    // A project without this remote is local-only: chats stay local.
    if !joy_core::vcs::remote_exists(root, &remote) {
        return;
    }
    let fetch_spec = format!(
        "+{}:{}",
        joy_chat_store::chat_ref::CHATS_REF,
        joy_chat_store::chat_ref::CHATS_TRACKING_REF
    );
    match joy_core::vcs::fetch_ref(root, &remote, &fetch_spec) {
        joy_core::vcs::RefTransfer::Done => {}
        joy_core::vcs::RefTransfer::Refused(stderr) => {
            // No remote chats yet: the normal first sync, nothing to merge.
            if !stderr.contains("couldn't find remote ref") {
                eprintln!(
                    "chats not fetched ({}); local state shown, the next send or read retries",
                    classify_sync_error(&stderr)
                );
            }
        }
        joy_core::vcs::RefTransfer::GitUnavailable(e) => {
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
    match joy_core::vcs::push_ref(root, &remote, &push_spec) {
        joy_core::vcs::RefTransfer::Done => {}
        joy_core::vcs::RefTransfer::Refused(stderr) => {
            eprintln!(
                "chat stays committed locally, push failed ({}); the next send or read retries",
                classify_sync_error(&stderr)
            );
        }
        joy_core::vcs::RefTransfer::GitUnavailable(e) => {
            eprintln!("chat stays committed locally, push failed (git unavailable: {e})")
        }
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

/// The chat a person means (JOY-026B-E7): `general`, the opaque id, the
/// joy id in any spelling (`1`, `0001`, `1-AB`, `MPS-CHAT-0001`, full),
/// or the chat's name. A counter two chats share needs the suffix; a
/// name two chats share needs the id - both are said, never guessed.
/// `30m`, `2h`, `3d` into a duration (the ages `show --since` takes).
fn parse_age(age: &str) -> Result<chrono::Duration> {
    let age = age.trim();
    let (number, unit) = age.split_at(age.len().saturating_sub(1));
    let n: i64 = number
        .parse()
        .map_err(|_| anyhow::anyhow!("{age}: say an age like 30m, 2h or 3d"))?;
    Ok(match unit {
        "m" => chrono::Duration::minutes(n),
        "h" => chrono::Duration::hours(n),
        "d" => chrono::Duration::days(n),
        _ => anyhow::bail!("{age}: say an age like 30m, 2h or 3d"),
    })
}

fn load_or_general(root: &std::path::Path, id: &str) -> Result<joy_chat::model::chat::Chat> {
    use joy_core::short_id::{matches, parse_input};
    if id == joy_chat_store::chats::GENERAL_CHAT_ID {
        return Ok(joy_chat_store::chats::ensure_general(
            root,
            chrono::Utc::now(),
        )?);
    }
    if let Some(chat) = joy_chat_store::chats::load_chat(root, id)? {
        return Ok(chat);
    }
    let chats = joy_chat_store::chats::load_chats(root)?;
    let prefix = joy_chat_store::chats::joy_id_prefix(root)?;
    let describe = |c: &joy_chat::model::chat::Chat| {
        format!(
            "{} ({})",
            c.joy_id.as_deref().unwrap_or(&c.id),
            c.title.as_deref().unwrap_or("untitled")
        )
    };
    if let Some(typed) = parse_input(&prefix, id) {
        let hits: Vec<_> = chats
            .iter()
            .filter(|c| {
                c.joy_id
                    .as_deref()
                    .is_some_and(|j| matches(&prefix, j, &typed))
            })
            .collect();
        match hits.len() {
            1 => return Ok(hits[0].clone()),
            0 => {}
            _ => anyhow::bail!(
                "{id} is ambiguous; say which: {}",
                hits.iter()
                    .map(|c| describe(c))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
    let wanted = id.trim();
    let named: Vec<_> = chats
        .iter()
        .filter(|c| {
            c.title
                .as_deref()
                .is_some_and(|t| t.trim().eq_ignore_ascii_case(wanted))
        })
        .collect();
    match named.len() {
        1 => Ok(named[0].clone()),
        0 => anyhow::bail!("no chat with id or name {id}"),
        _ => anyhow::bail!(
            "{id} names {} chats; use the id: {}",
            named.len(),
            named
                .iter()
                .map(|c| describe(c))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// The chat key this SESSION carries.
///
/// An AI acts with a token and never with a passphrase: its delegation
/// key rides in the session, and that key IS its identity, for chats as
/// for zones. Zone keys already worked this way; chats did not, so an AI
/// saw "No chats" in rooms it is a member of (JOY-023E-68). `--session`
/// is copied into the environment before any command runs, so reading it
/// here covers both ways of passing one.
fn session_chat_seed() -> Option<[u8; 32]> {
    let env_value = std::env::var("JOY_SESSION").ok()?;
    let (_sid, _ephemeral, delegation) =
        joy_core::auth::session::parse_session_env_full(&env_value)?;
    delegation
}

/// The chat seed a standing session of the acting person carries, if the
/// session is still valid for this project and member.
fn stored_session_seed(root: &std::path::Path) -> Option<[u8; 32]> {
    let identity = joy_core::identity::resolve_identity(root).ok()?;
    if !identity.authenticated {
        return None;
    }
    let project_id = joy_core::auth::session::project_id(root).ok()?;
    let token = joy_core::auth::session::load_session(&project_id, &identity.member).ok()??;
    if token.claims.expires <= chrono::Utc::now() {
        return None;
    }
    let bytes = hex::decode(token.chat_seed.as_deref()?).ok()?;
    bytes.try_into().ok()
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
    // A person with a standing session (joy auth) brings the seed the
    // login cached (JOY-0269-BC): the session suffices, as for every
    // other command; the passphrase is asked only without one.
    if passphrase.is_none() && !stdin {
        if let Some(seed) = stored_session_seed(root) {
            joy_chat_store::writer::set_seed(Some(seed));
            return Ok(());
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
    let started = std::time::Instant::now();
    let is_send = matches!(args.command, ChatCommand::Send { .. });
    // A read fetches first, so it shows the forge's state. A write does
    // NOT (JOY-026C-34): it appends locally and pushes; only a refused
    // push fetches, unites and pushes again - one contact per message in
    // the common case instead of three.
    if is_read {
        sync_ref(&root);
    }
    let is_show = matches!(args.command, ChatCommand::Show { .. });
    let result = run_command(&root, args.command);
    // a show moves the read marker, so it delivers like a write
    if result.is_ok() && (!is_read || is_show) {
        deliver_ref(&root);
    }
    if result.is_ok() && is_send {
        // what the person did, and how long the WHOLE of it took, the
        // push to the forge included - never the chat's title, which
        // read like a technology (JOY-026A-F2)
        println!("message sent ({} ms)", started.elapsed().as_millis());
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
                    c.joy_id.as_deref().unwrap_or(&c.id),
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
            let identity = acting_identity(root)?;
            let me = identity.member.clone();
            let mut chat = load_or_general(root, &id)?;
            let text = text.join(" ");
            if text.trim().is_empty() {
                anyhow::bail!("nothing to send");
            }
            // An AI turn is started by the client that SENDS the message,
            // under the sender's delegation (the app: web on the platform
            // host, desktop on the local host). This command has no chat
            // session and no turn host, so an AI it addresses would never
            // answer - and the sender would never learn why (JOY-0270-40,
            // Horst 2026-08-30: refuse, do not send into the void).
            let ais: Vec<String> = joy_core::store::load_project(root)?
                .members()
                .map(|(id, _)| id.clone())
                .filter(|id| id.starts_with("ai:"))
                .collect();
            if let Some(ai) = joy_chat::mentions::leading_mentions(&text, &ais).first() {
                anyhow::bail!(
                    "@{} answers only when addressed from the app (an AI turn runs under the sender's chat session there); nothing sent. Address {} in the app, or send the message without the mention.",
                    joy_chat::mentions::alias(ai),
                    joy_chat::mentions::alias(ai)
                );
            }
            match identity.delegated_by {
                // an AI acting for a person says so on the message, the
                // way its turn replies do (the app shows "delegated by")
                Some(by) => {
                    joy_chat_store::chats::append_ai_reply(
                        root,
                        &mut chat,
                        me,
                        text,
                        chrono::Utc::now(),
                        None,
                        Some(by.id().to_string()),
                        None,
                        None,
                        None,
                    )?;
                }
                None => {
                    joy_chat_store::chats::append_message(
                        root,
                        &mut chat,
                        me,
                        text,
                        chrono::Utc::now(),
                    )?;
                }
            }
            // the confirmation is printed by run() AFTER the push to the
            // forge, with the honest time (JOY-026A-F2)
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
        ChatCommand::Info { id } => {
            let me = acting_member(root)?;
            let chat = load_or_general(root, &id)?;
            // General and team chats carry no explicit list: everyone in
            // the project is in (JOY-026E-0F: "0 participant(s)" for General)
            let participants = joy_chat_store::chats::effective_participants(root, &chat)?;
            println!(
                "{} — {} participant(s), {} message(s)",
                chat.title.as_deref().unwrap_or("(untitled)"),
                participants.len(),
                chat.messages.len(),
            );
            println!("your unread: {}", chat.unread_count(me.id()));
            if let Some(last) = chat.messages.last() {
                let readers = chat.read_by(last);
                println!(
                    "latest read by {}/{}: {}",
                    readers.len(),
                    participants.len(),
                    if readers.is_empty() {
                        "(nobody yet)".to_string()
                    } else {
                        readers.join(", ")
                    },
                );
            }
            for p in &participants {
                println!("  {:<28} {} unread", p.id(), chat.unread_count(p.id()));
            }
        }
        ChatCommand::Show {
            id,
            unread,
            last,
            since,
        } => {
            // The same resolver as every other chat command (JOY-0271-34):
            // number, MPS-CHAT-0010, name or general.
            let me = acting_member(root)?;
            let mut chat = load_or_general(root, &id)?;
            let participants = joy_chat_store::chats::effective_participants(root, &chat)?;
            let cutoff = match since.as_deref() {
                Some(age) => Some(chrono::Utc::now() - parse_age(age)?),
                None => None,
            };
            let unread_from = unread.then(|| chat.unread_count(me.id()));
            let total = chat.messages.len();
            let shown: Vec<&joy_chat::model::chat::ChatMessage> = chat
                .messages
                .iter()
                .enumerate()
                .filter(|(i, m)| {
                    cutoff.is_none_or(|c| m.at >= c) && unread_from.is_none_or(|n| *i + n >= total)
                })
                .map(|(_, m)| m)
                .collect();
            let shown: Vec<_> = match last {
                Some(n) if n < shown.len() => shown[shown.len() - n..].to_vec(),
                _ => shown,
            };
            println!(
                "{} — {} participant(s), {} of {} message(s)",
                chat.title.as_deref().unwrap_or("(untitled)"),
                participants.len(),
                shown.len(),
                total
            );
            for m in shown {
                let by = match &m.delegated_by {
                    Some(by) => format!("{} (delegated by {by})", m.author.id()),
                    None => m.author.id().to_string(),
                };
                println!("{}  {}  {}", m.at.format("%Y-%m-%d %H:%M"), by, m.text);
                // Non-text parts are named, never dropped (JOY-024C-97):
                // the CLI cannot show an image, but it must say one is
                // there.
                for part in &m.parts {
                    use joy_chat::model::chat::MessagePart;
                    let label = match part {
                        MessagePart::Text { text } => text.clone(),
                        MessagePart::Image { label, .. }
                        | MessagePart::Audio { label, .. }
                        | MessagePart::Resource { label, .. } => label.clone(),
                        MessagePart::ResourceLink { uri, label } => {
                            if label.is_empty() {
                                uri.clone()
                            } else {
                                format!("{label} <{uri}>")
                            }
                        }
                    };
                    println!("                    [{}] {}", part.kind_word(), label);
                }
            }
            // showing is reading, as in the apps: the read marker moves
            // and rides to the forge with the next delivery
            joy_chat_store::chats::mark_read(root, &mut chat, &me, chrono::Utc::now())?;
        }
    }
    Ok(())
}
