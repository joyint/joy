// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! THE chat turn loop (JI-0179-4F): one engine for every host.
//!
//! Until 2026-07 this loop existed twice, once in the platform's server
//! (chat turns in project containers) and once in the desktop app's Rust
//! shell (chat turns over local ACP tools). Only fragments of the rules
//! were shared, and every fix that landed in one copy silently made the
//! other one poorer; three operator findings in one day were all this
//! class (APP-FAIL.md in the umbrella records the full accounting).
//!
//! The rule now: hosts never decide anything product-shaped. This module
//! owns the loop, the notices a person reads, the waiting-marker
//! ordering, the reply hygiene and the execution record. A host
//! implements [`TurnHost`], which is deliberately narrow: load a chat,
//! commit a write, run one agent turn, deliver an ephemeral event, and
//! answer capability questions (is this AI usable, is there budget).
//! Everything a host returns for a refusal is an ENUM, never a sentence;
//! the engine formats the one sentence both apps show.

use std::path::PathBuf;

use joy_chat::model::chat::{Chat, ChatMessage, MessageKind};
use joy_chat::model::AgentMode;
use joy_core::model::config::InteractionLevel;

use crate::chat_turns::{self, TurnDecision};

/// Hard backstop for AI-to-AI rounds, well above the chain guard's own
/// bound (the guard in [`chat_turns::decide`] moderates first).
const MAX_ROUNDS: u8 = 8;

/// The project and conversation a loop run works on.
pub struct EngineCtx {
    /// The project root (the checkout on the platform, the opened
    /// directory on the desktop).
    pub root: PathBuf,
    pub chat_id: String,
    /// The sender of the triggering message: the delegator of every turn
    /// this run performs, and the attribution on every reply.
    pub sender: String,
}

/// A write the engine wants committed. The HOST owns the transaction
/// around it (gate lock, git commit, live fan-out, forge note on the
/// platform; plain file append on the desktop); the engine owns every
/// field in here.
pub struct AppendSpec {
    pub member: String,
    pub text: String,
    /// true: a centered notice; false: the AI's reply message.
    pub notice: bool,
    pub delegated_by: Option<String>,
    pub turn_ms: Option<u32>,
    pub tool_steps: Option<u32>,
    /// The persisted activity block + execution record (turn_meta).
    pub details: Option<String>,
}

/// A committed write, as the chat file now holds it.
pub struct Appended {
    pub seq: u64,
    pub message: ChatMessage,
}

/// Why an AI member cannot take a turn for this sender. Conditions are
/// host capabilities (a local host knows about installed binaries, the
/// platform knows about keys); the SENTENCES for them live in the engine.
pub enum Usability {
    Usable,
    /// Platform: the sender has no usable API key for this member.
    NoKey,
    /// Desktop: the tool is not installed on this machine.
    NotInstalled {
        hint: String,
    },
    /// Desktop: the tool is not activated in this project.
    NotActivated,
}

/// The waiting-marker signal (JAPP-0129-A7 / JP-0097-65): `Start` opens
/// the one-line skeleton for the member, `Done` closes it. The engine
/// guarantees the ordering (Done AFTER the append, JP-0093-51), the host
/// only transports the event (chat bus or Tauri event).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    Start,
    Done,
}

/// One agent turn, as the engine requests it from the host.
pub struct TurnRequest<'a> {
    pub member: &'a str,
    /// The full transcript prompt; a fresh session replays this.
    pub prompt: String,
    /// The delta since the member's last turn; a live session prefers it
    /// (JP-0085-F4). None when there is nothing new.
    pub prompt_delta: Option<String>,
    /// The one-way derived agent mode for the ACP boundary.
    pub mode: AgentMode,
    /// The effective interaction level the turn runs under.
    pub level: InteractionLevel,
}

/// The platform's money answer for a finished turn: the member's spend
/// after this turn against the cap shown in the info popover. A local
/// host has no budget and returns None from its outcome.
pub struct BudgetSnapshot {
    pub spent_cents: u64,
    pub cap_cents: Option<u64>,
}

/// What one agent turn produced. Hosts fill what they can observe and
/// leave the rest None; the engine never invents a value.
#[derive(Default)]
pub struct TurnOutcome {
    pub reply: String,
    /// The activity block (v1 details JSON) if the host collected one.
    pub details: Option<String>,
    pub tool_steps: u32,
    pub cost_cents: Option<u64>,
    pub tokens: Option<u64>,
    /// The model that answered, when the host routed one.
    pub model: Option<String>,
    /// The turn was stopped early by a per-message spend cap.
    pub capped: bool,
    pub budget: Option<BudgetSnapshot>,
}

/// The pre-turn capability check, BEFORE the waiting marker opens: a
/// refusal here must not flash a skeleton (the platform refuses before
/// spending, JI-014A).
pub enum Preflight {
    Ready,
    BudgetExhausted,
}

/// What a host provides. Every method is a capability or a transaction;
/// none of them decides wording, ordering or policy.
pub trait TurnHost {
    fn load_chat(&self, chat_id: &str) -> Option<Chat>;
    /// Run the shared mention-add ([`chat_turns::add_mentioned_ais`])
    /// inside the host's write transaction and deliver the change (live
    /// fan-out, forge note). Returns whether participants changed.
    fn add_mentioned_ais(&self, chat_id: &str, newest: &ChatMessage) -> bool;
    /// Commit one write. See [`AppendSpec`].
    fn append(&self, chat_id: &str, spec: AppendSpec) -> Result<Appended, String>;
    /// Deliver the ephemeral waiting marker. `marker_id` is unique per
    /// turn so client-side dedupe never swallows a later one.
    fn publish_pending(&self, chat_id: &str, member: &str, kind: Pending, marker_id: &str);
    /// May this member act for the run's sender?
    fn usable(&self, member: &str) -> Usability;
    /// The sender's personal overall level for this member, when the
    /// host stores one (platform DB); None otherwise.
    fn personal_level(&self, member: &str) -> Option<InteractionLevel>;
    /// The pre-turn budget check; a local host is always [`Preflight::Ready`].
    fn preflight(&self, member: &str) -> Preflight;
    /// Run ONE agent turn. Err carries the raw adapter error; the engine
    /// turns it into the human notice.
    fn run_turn(&self, req: &TurnRequest) -> Result<TurnOutcome, String>;
}

/// Run the AI turns a new message may have triggered: every addressed AI
/// participant answers with the full transcript as context; AI-to-AI
/// mentions loop until the joy-core chain guard posts the moderation
/// notice. Returns the committed REPLY messages (notices are committed
/// too but not returned; clients refresh them over their stream).
pub fn run_chat_turns(ctx: &EngineCtx, host: &dyn TurnHost) -> Vec<Appended> {
    let mut appended = Vec::new();
    for _round in 0..MAX_ROUNDS {
        let Some(mut chat) = host.load_chat(&ctx.chat_id) else {
            break;
        };
        // General carries "everyone" implicitly; the turn logic needs the
        // resolved list. Best effort: an unresolved list must not kill
        // the turns.
        if let Ok(participants) = joy_chat::chats::effective_participants(&ctx.root, &chat) {
            chat.participants = participants;
        }
        let Some(newest) = chat.messages.last().cloned() else {
            break;
        };
        // A human's @mention of a project AI pulls it into the chat
        // first (the reason a fresh personal chat can talk to @claude at
        // all). Participants changed: reload for THIS round but keep
        // deciding on the human message (the add's notice is newer in
        // the file and would silence everyone).
        if host.add_mentioned_ais(&ctx.chat_id, &newest) {
            if let Some(fresh) = host.load_chat(&ctx.chat_id) {
                chat = fresh;
                if let Ok(participants) = joy_chat::chats::effective_participants(&ctx.root, &chat)
                {
                    chat.participants = participants;
                }
            }
        }
        let ai_members: Vec<String> = chat
            .participants
            .iter()
            .map(|p| p.id().to_string())
            .filter(|id| id.starts_with("ai:"))
            .collect();
        let mut acted = false;
        for member in ai_members {
            match chat_turns::decide(&chat, &newest, &member) {
                TurnDecision::Silent => {}
                TurnDecision::NeedsModeration => {
                    if !chat_turns::moderation_already_posted(&chat)
                        && host
                            .append(
                                &ctx.chat_id,
                                notice(&member, chat_turns::MODERATION_NOTICE.to_string()),
                            )
                            .is_ok()
                    {
                        acted = true;
                    }
                }
                TurnDecision::Respond => {
                    let alias = chat_turns::alias(&member);
                    // The capability gate, with ONE sentence per
                    // condition, wherever the turn runs.
                    if let Some(note) = usability_notice(alias, &host.usable(&member), ctx) {
                        if !recently_noticed(&chat, &note)
                            && host.append(&ctx.chat_id, notice(&member, note)).is_ok()
                        {
                            acted = true;
                        }
                        continue;
                    }
                    // The level of THIS turn (JI-0166-D8 §5): the sender
                    // is the delegator, resolved by the one shared rule
                    // (ADR-025 order); the ACP boundary gets the one-way
                    // derived agent mode.
                    let level = joy_chat::turn_meta::resolve_effective_level(
                        &ctx.root,
                        &chat,
                        &member,
                        &ctx.sender,
                        host.personal_level(&member),
                    );
                    let request = TurnRequest {
                        member: &member,
                        prompt: chat_turns::context_prompt(&chat, &member),
                        prompt_delta: chat_turns::delta_prompt(&chat, &member),
                        mode: joy_chat::model::agent_mode::from_level(level),
                        level,
                    };
                    // Refuse BEFORE spending and BEFORE the skeleton
                    // opens (JI-014A): a budget refusal must not flash a
                    // waiting indicator.
                    if let Preflight::BudgetExhausted = host.preflight(&member) {
                        let note = format!(
                            "@{alias} has reached its monthly budget for this project. Raise it in Settings → AI members to continue."
                        );
                        if !recently_noticed(&chat, &note)
                            && host.append(&ctx.chat_id, notice(&member, note)).is_ok()
                        {
                            acted = true;
                        }
                        continue;
                    }
                    // The reply-in-preparation marker (JP-0097-65):
                    // non-streaming adapters return only the final text,
                    // so without a signal the room looks dead while the
                    // model works. Done fires AFTER the append
                    // (JP-0093-51) so the reply fills the skeleton and
                    // the withheld/empty/failed branches still close it.
                    let marker_id = format!("pending-{}-{}", member, uuid::Uuid::new_v4().simple());
                    host.publish_pending(&ctx.chat_id, &member, Pending::Start, &marker_id);
                    let started = std::time::Instant::now();
                    let outcome = host.run_turn(&request);
                    let capped = outcome.as_ref().map(|o| o.capped).unwrap_or(false);
                    match outcome {
                        Ok(out) if !out.reply.trim().is_empty() => {
                            let turn_ms =
                                started.elapsed().as_millis().min(u32::MAX as u128) as u32;
                            // The execution record (JI-014A/JI-0162):
                            // level, model, cost, tokens, and the budget
                            // snapshot where one exists, folded into the
                            // details for the per-message info popover.
                            let details = joy_chat::turn_meta::augment_details(
                                out.details,
                                &joy_chat::turn_meta::TurnMeta {
                                    model: out.model.as_deref(),
                                    cost_cents: out.cost_cents,
                                    tokens: out.tokens,
                                    interaction_level: Some(&level.to_string()),
                                    spent_cents: out.budget.as_ref().map(|b| b.spent_cents),
                                    cap_cents: out.budget.as_ref().and_then(|b| b.cap_cents),
                                },
                            );
                            // Nothing tool-shaped may surface as chat
                            // text (JAPP-010D-B0); a blob-only reply is
                            // withheld with an honest notice.
                            let committed = match chat_turns::sanitize_reply(&out.reply) {
                                Some(clean) => host
                                    .append(
                                        &ctx.chat_id,
                                        AppendSpec {
                                            member: member.clone(),
                                            text: clean,
                                            notice: false,
                                            delegated_by: Some(ctx.sender.clone()),
                                            turn_ms: Some(turn_ms),
                                            tool_steps: (out.tool_steps > 0)
                                                .then_some(out.tool_steps),
                                            details,
                                        },
                                    )
                                    .map(|a| appended.push(a))
                                    .is_ok(),
                                None => {
                                    let note = format!(
                                        "@{alias} answered with raw tool-call data; the reply was withheld"
                                    );
                                    host.append(&ctx.chat_id, notice(&member, note)).is_ok()
                                }
                            };
                            if committed {
                                acted = true;
                            }
                        }
                        Ok(_) => {}
                        Err(raw) => {
                            // The raw adapter error (container names,
                            // tag wrappers) never reaches the chat; the
                            // human notice survives a refresh.
                            let note = humanize_turn_error(alias, &raw);
                            if host.append(&ctx.chat_id, notice(&member, note)).is_ok() {
                                acted = true;
                            }
                        }
                    }
                    host.publish_pending(&ctx.chat_id, &member, Pending::Done, &marker_id);
                    if capped {
                        // The turn ran partially and its spend is
                        // recorded; say so plainly so an abruptly short
                        // reply is not a mystery.
                        let note = format!(
                            "@{alias} stopped at the per-message spend limit. Continue with a smaller step, or raise the limit in Settings."
                        );
                        if host.append(&ctx.chat_id, notice(&member, note)).is_ok() {
                            acted = true;
                        }
                    }
                }
            }
        }
        if !acted {
            break;
        }
    }
    appended
}

fn notice(member: &str, text: String) -> AppendSpec {
    AppendSpec {
        member: member.to_string(),
        text,
        notice: true,
        delegated_by: None,
        turn_ms: None,
        tool_steps: None,
        details: None,
    }
}

/// The one sentence per unusable condition. None: the member is usable.
fn usability_notice(alias: &str, usability: &Usability, ctx: &EngineCtx) -> Option<String> {
    match usability {
        Usability::Usable => None,
        Usability::NoKey => Some(format!(
            "@{alias} is not configured for {}. An API key in Settings makes it usable.",
            ctx.sender
        )),
        Usability::NotInstalled { hint } => {
            Some(format!("@{alias} is not set up on this machine ({hint})"))
        }
        Usability::NotActivated => Some(format!(
            "@{alias} is not activated in this project — Settings → Local AI → Active"
        )),
    }
}

/// A notice is posted once, not per round: the same text within the last
/// four messages counts as already said.
fn recently_noticed(chat: &Chat, text: &str) -> bool {
    chat.messages
        .iter()
        .rev()
        .take(4)
        .any(|m| m.kind == MessageKind::Notice && m.text == text)
}

/// Turn a raw adapter error into a human chat notice (operator
/// 2026-07-20). The adapter emits container-prefixed, tag-wrapped text
/// like `acp chat turn in joyint-project-…: <vibe_stop_event>Price limit
/// exceeded: $0.07 > $0.05</vibe_stop_event>`; none of that belongs in a
/// chat. A spend/budget stop points at Settings without inventing a
/// figure; anything else becomes a short honest line.
pub fn humanize_turn_error(alias: &str, raw: &str) -> String {
    // drop the "…chat turn in <container>: " prefix
    let after = raw
        .find("chat turn in ")
        .and_then(|i| raw[i..].find(": ").map(|j| &raw[i + j + 2..]))
        .unwrap_or(raw);
    // strip any <tag> … </tag> wrappers, then collapse whitespace
    let mut cleaned = String::new();
    let mut in_tag = false;
    for c in after.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => cleaned.push(c),
            _ => {}
        }
    }
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = cleaned.to_lowercase();
    if lower.contains("limit exceeded") || lower.contains("price limit") || lower.contains("budget")
    {
        return format!(
            "@{alias} stopped before finishing: this turn hit a spend limit. \
             Adjust this agent's budget in Settings → AI members to continue."
        );
    }
    let short: String = cleaned.chars().take(240).collect();
    if short.is_empty() {
        format!("@{alias} could not finish this turn. Please try again.")
    } else {
        format!("@{alias} could not finish this turn: {short}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Moved with the function from platform/src/api/chat_ai.rs
    // (JI-0179-4F step 1): the wording is product surface and tested
    // where it lives now.
    #[test]
    fn humanize_strips_container_prefix_and_tags() {
        let raw = "acp chat turn in joyint-project-abc: <vibe_stop_event>ReadTimeout on upstream</vibe_stop_event>";
        assert_eq!(
            humanize_turn_error("vibe", raw),
            "@vibe could not finish this turn: ReadTimeout on upstream"
        );
    }

    #[test]
    fn a_price_limit_stop_points_at_settings_without_a_figure() {
        let raw = "acp chat turn in joyint-project-5be53877: \
                   <vibe_stop_event>Price limit exceeded: $0.0728 > $0.05</vibe_stop_event>";
        let msg = humanize_turn_error("vibe", raw);
        // no container name, no tags, no raw dollar figures
        assert!(!msg.contains("joyint-project"), "leaked container: {msg}");
        assert!(
            !msg.contains('<') && !msg.contains('>'),
            "leaked tags: {msg}"
        );
        assert!(!msg.contains("$0.05"), "echoed the raw limit: {msg}");
        // no fabricated ceiling; just the honest spend-limit line + pointer
        assert!(msg.contains("spend limit"), "missing the reason: {msg}");
        assert!(
            msg.contains("Settings"),
            "missing the settings pointer: {msg}"
        );
        assert!(msg.starts_with("@vibe"), "missing the mention: {msg}");
    }
    #[test]
    fn a_generic_error_degrades_to_a_short_tag_free_line() {
        let msg = humanize_turn_error(
            "vibe",
            "chat turn in joyint-project-x: <err>connection reset by peer</err>",
        );
        assert_eq!(
            msg,
            "@vibe could not finish this turn: connection reset by peer"
        );
    }

    #[test]
    fn humanize_never_posts_an_empty_reason() {
        assert_eq!(
            humanize_turn_error("qwen", "<only><tags></tags></only>"),
            "@qwen could not finish this turn. Please try again."
        );
    }
}
