// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! THE chat turn core (JI-0179-4F, JOY-0249-D2): one codebase for every
//! host.
//!
//! The turn LOOP lives in the sealed chat client (sealedChatProvider),
//! shared by the web and the desktop app, because only the key holder
//! can read the conversation (operator decision 2026-07-28). What a host
//! contributes is exactly ONE turn: run the agent, stream its live
//! activity, and return the outcome. Until 2026-08 each host had its own
//! copy of that too — its own level fallback, its own marker format, its
//! own outcome assembly, its own activity field mapping — and every fix
//! that landed in one copy silently made the other one poorer.
//!
//! The rule: hosts never decide anything product-shaped. This module
//! owns the per-turn choreography ([`run_host_turn`]), the live-activity
//! wire shape ([`WireActivity`]), the delivery contract ([`TurnSink`]:
//! everything a turn streamed is DELIVERED before its result returns,
//! so a stray trailing chunk can never overtake the reply), the level
//! resolution, the execution record and every sentence a person reads.
//! A host implements only the last mile: publish one [`WireActivity`]
//! on its transport, and run one agent.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use joy_chat::model::AgentMode;
use joy_core::model::config::InteractionLevel;

use crate::chat_turns;

/// Live activity of a RUNNING turn (JI-0172-EE): the agent's streamed
/// chunks, thoughts and tool calls. One vocabulary for every transport;
/// ephemeral like the waiting marker it upgrades — the persisted reply
/// is the only truth.
#[derive(Debug, Clone)]
pub enum TurnActivity {
    /// A piece of the reply text.
    Chunk { text: String },
    /// A piece of the reasoning text.
    Thought { text: String },
    /// A tool call started or changed status.
    Tool {
        id: String,
        title: String,
        status: String,
    },
}

impl TurnActivity {
    /// The wire kind, identical on the bus and the Tauri event, so the
    /// channel needs exactly one switch.
    pub fn kind(&self) -> &'static str {
        match self {
            TurnActivity::Chunk { .. } => "turn-chunk",
            TurnActivity::Thought { .. } => "turn-thought",
            TurnActivity::Tool { .. } => "turn-tool",
        }
    }
}

/// The ONE wire shape of a live turn event, on every transport
/// (JOY-0249-D2): the chat bus wraps it in its ChatEvent, the Tauri
/// event carries it directly; the client folds both into the same
/// TurnActivityEvent. Field reuse by design: `text` is the chunk or
/// thought text or the tool title, `tool` the tool-call id, `payload`
/// the tool status, `id` the waiting-marker id of a `pending` event.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireActivity {
    /// "pending" | "turn-chunk" | "turn-thought" | "turn-tool"
    pub kind: String,
    /// The waiting-marker id (pending events only; unique per turn so
    /// client-side dedupe never swallows a later one).
    pub id: String,
    pub text: String,
    pub tool: String,
    pub payload: String,
}

impl WireActivity {
    /// The waiting marker that opens the reply-in-preparation skeleton
    /// (JAPP-0129-A7). Closed by the reply that takes its place, never
    /// by a signal (JAPP-0169-78).
    pub fn pending(marker_id: &str) -> Self {
        WireActivity {
            kind: "pending".into(),
            id: marker_id.to_string(),
            text: String::new(),
            tool: String::new(),
            payload: String::new(),
        }
    }

    /// One streamed activity event on the wire.
    pub fn of(activity: &TurnActivity) -> Self {
        let (text, tool, payload) = match activity {
            TurnActivity::Chunk { text } | TurnActivity::Thought { text } => {
                (text.clone(), String::new(), String::new())
            }
            TurnActivity::Tool { id, title, status } => (title.clone(), id.clone(), status.clone()),
        };
        WireActivity {
            kind: activity.kind().into(),
            id: String::new(),
            text,
            tool,
            payload,
        }
    }
}

/// How live turn events reach the room. The ONE host-specific part of
/// the streaming path: the platform awaits a chat-bus publish, the
/// desktop wraps its (synchronous) Tauri emit.
///
/// The delivery CONTRACT is what kills the stray-chunk class
/// (JOY-0249-D2): the shared lane awaits every delivery and drains the
/// stream before a turn's result returns, so nothing of a turn can
/// arrive after its reply. Implementations swallow their own transport
/// errors — a lost ephemeral event must never fail the turn.
pub trait TurnSink: Send + Sync {
    fn deliver<'a>(
        &'a self,
        activity: WireActivity,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

/// Why an AI member cannot take a turn for this sender. Conditions are
/// host capabilities (a local host knows about installed binaries, the
/// platform knows about keys); the SENTENCES for them live here.
pub enum Usability {
    Usable,
    /// Platform: the sender has no usable API key for this member.
    NoKey,
    /// Platform: the sender has not delegated to this member, so a turn
    /// could not act under their name. The AI never acts under anyone
    /// else's (operator rule, 2026-07-29: no exception).
    NotDelegated,
    /// Desktop: the tool is not installed on this machine.
    NotInstalled {
        hint: String,
    },
    /// Desktop: the tool is not activated in this project.
    NotActivated,
}

/// One agent turn, as the core requests it from the host's agent
/// plumbing (docker lane on the platform, PATH lane on the desktop).
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
    /// The waiting-marker id of this turn. The shared lane delivers the
    /// pending event under this id before the first prompt byte, on the
    /// same ordered wire as the chunks.
    pub marker_id: String,
}

/// The platform's money answer for a finished turn: the member's spend
/// after this turn against the cap shown in the info popover. A local
/// host has no budget and returns None from its outcome.
pub struct BudgetSnapshot {
    pub spent_cents: u64,
    pub cap_cents: Option<u64>,
}

/// What one agent turn produced. Hosts fill what they can observe and
/// leave the rest None; the core never invents a value.
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
    /// How many permission round-trips this turn was REFUSED. An agent
    /// that is not allowed to do what it planned typically ends its turn
    /// without a word (measured with vibe-acp 2.22, JAPP-0152-81).
    pub denied: u32,
    pub budget: Option<BudgetSnapshot>,
}

/// The pre-turn capability check, BEFORE the waiting marker opens: a
/// refusal here must not flash a skeleton (the platform refuses before
/// spending, JI-014A).
pub enum Preflight {
    Ready,
    BudgetExhausted,
}

/// Everything ONE host turn starts from. The prompt and the chat-scoped
/// level override come from the client, which alone holds the chat key;
/// the rest is host capability.
pub struct HostTurnSpec<'a> {
    pub root: &'a Path,
    pub member: &'a str,
    pub prompt: String,
    pub prompt_delta: Option<String>,
    /// The caller's per-chat level choice (ADR-025 rank 1); only the
    /// client can read it, so it sends it.
    pub level_override: Option<InteractionLevel>,
    /// The caller's personal overall level for this member, when the
    /// host stores one (platform DB); None on the desktop.
    pub personal_level: Option<InteractionLevel>,
}

/// What the client gets back from one host turn: either the reply with
/// its execution record, or the one human sentence why there is none.
/// Serializes camelCase: it IS the wire shape of the desktop's Tauri
/// answer, and the platform's proto mirrors the same fields.
#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostTurnOutcome {
    pub reply: String,
    pub notice: String,
    pub turn_ms: u32,
    pub tool_steps: u32,
    pub details: String,
    /// The model the agent reported, for the host's postmortem line.
    pub model: String,
}

/// The host-side share of the ADR-025 level order: chat override (from
/// the client) > the caller's personal overall > the member's default >
/// the project default. The chat-scoped rank lives with the key holder;
/// everything below is store state both hosts can read — and used to
/// resolve DIFFERENTLY per host until JOY-0249-D2's audit.
pub fn host_turn_level(
    root: &Path,
    member: &str,
    level_override: Option<InteractionLevel>,
    personal_level: Option<InteractionLevel>,
) -> InteractionLevel {
    level_override
        .or(personal_level)
        .or_else(|| {
            joy_core::store::load_project(root)
                .ok()
                .and_then(|p| p.member_by_key(member).and_then(|m| m.interaction_level))
        })
        .unwrap_or_else(|| joy_core::store::load_interaction_level_defaults(root).default)
}

/// Run ONE host turn: resolve the level, mint the waiting marker, run
/// the agent, assemble the outcome. The choreography both hosts used to
/// copy by hand — level fallback, marker format, timing, execution
/// record, error wording — lives here exactly once. `run_agent` is the
/// host's agent plumbing; everything it streams rides the [`TurnSink`]
/// the host handed to its lane, delivered in order and drained before
/// this returns.
pub fn run_host_turn(
    spec: HostTurnSpec,
    run_agent: impl FnOnce(&TurnRequest) -> Result<TurnOutcome, String>,
) -> HostTurnOutcome {
    let alias = chat_turns::alias(spec.member);
    let level = host_turn_level(
        spec.root,
        spec.member,
        spec.level_override,
        spec.personal_level,
    );
    let request = TurnRequest {
        member: spec.member,
        prompt: spec.prompt,
        prompt_delta: spec.prompt_delta,
        mode: joy_chat::model::agent_mode::from_level(level),
        level,
        // unique per turn: the skeleton keys on it and a later one must
        // never be swallowed by client-side dedupe
        marker_id: format!("pending-{}-{}", spec.member, uuid::Uuid::new_v4().simple()),
    };
    let started = std::time::Instant::now();
    // Nothing closes the waiting row from here: the message that takes
    // its place does (JAPP-0169-78). This side being finished is not the
    // room having the answer — the client still has to seal and commit.
    match run_agent(&request) {
        Ok(out) => HostTurnOutcome {
            turn_ms: started.elapsed().as_millis().min(u32::MAX as u128) as u32,
            tool_steps: out.tool_steps,
            // level, model, cost, tokens and the budget snapshot: the
            // execution record the info popover reads (JAPP-016A-E0).
            details: turn_details(&out, level).unwrap_or_default(),
            model: out.model.clone().unwrap_or_default(),
            reply: out.reply,
            ..Default::default()
        },
        // The adapter's raw text is container-prefixed and tag-wrapped;
        // this module owns the wording both hosts show (JP-00A8-C9).
        Err(raw) => HostTurnOutcome {
            notice: humanize_turn_error(alias, &raw),
            ..Default::default()
        },
    }
}

/// The one sentence per unusable condition. None: the member is usable.
/// `sender` is the person the turn would act for.
pub fn usability_notice(alias: &str, usability: &Usability, sender: &str) -> Option<String> {
    match usability {
        Usability::Usable => None,
        Usability::NoKey => Some(format!(
            "@{alias} is not configured for {sender}. An API key in Settings makes it usable."
        )),
        Usability::NotDelegated => Some(format!(
            "@{alias} is not delegated by {sender}. Delegate in Settings → AI members, \
             then it can act for you."
        )),
        Usability::NotInstalled { hint } => {
            Some(format!("@{alias} is not set up on this machine ({hint})"))
        }
        Usability::NotActivated => Some(format!(
            "@{alias} is not activated in this project. Settings, Local AI, Active."
        )),
    }
}

/// The execution record for ONE finished turn (JI-014A / JI-0162):
/// level, model, cost, tokens and the budget snapshot where a host has
/// one, folded into the details the info popover reads.
pub fn turn_details(out: &TurnOutcome, level: InteractionLevel) -> Option<String> {
    joy_chat_store::turn_meta::augment_details(
        out.details.clone(),
        &joy_chat_store::turn_meta::TurnMeta {
            model: out.model.as_deref(),
            cost_cents: out.cost_cents,
            tokens: out.tokens,
            interaction_level: Some(&level.to_string()),
            spent_cents: out.budget.as_ref().map(|b| b.spent_cents),
            cap_cents: out.budget.as_ref().and_then(|b| b.cap_cents),
        },
    )
}

/// What the room is told when a member's money is spent.
pub fn budget_notice(alias: &str) -> String {
    format!(
        "@{alias} has reached its monthly budget for this project. \
         Raise it in Settings → AI members to continue."
    )
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

    #[test]
    fn a_host_turn_assembles_the_record_and_the_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let spec = HostTurnSpec {
            root: dir.path(),
            member: "ai:vibe@joy",
            prompt: "full".into(),
            prompt_delta: Some("delta".into()),
            level_override: Some(InteractionLevel::Autonomous),
            personal_level: None,
        };
        let outcome = run_host_turn(spec, |req| {
            // the choreography hands the agent everything resolved
            assert_eq!(req.level, InteractionLevel::Autonomous);
            assert!(req.marker_id.starts_with("pending-ai:vibe@joy-"));
            assert_eq!(req.prompt_delta.as_deref(), Some("delta"));
            Ok(TurnOutcome {
                reply: "da".into(),
                tool_steps: 2,
                model: Some("mistral-medium".into()),
                tokens: Some(9),
                ..Default::default()
            })
        });
        assert_eq!(outcome.reply, "da");
        assert_eq!(outcome.tool_steps, 2);
        assert_eq!(outcome.model, "mistral-medium");
        assert!(outcome.notice.is_empty());
        assert!(
            outcome.details.contains("autonomous"),
            "{}",
            outcome.details
        );
        assert!(outcome.details.contains("mistral-medium"));
    }

    #[test]
    fn a_failed_agent_run_becomes_the_one_human_sentence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let spec = HostTurnSpec {
            root: dir.path(),
            member: "ai:vibe@joy",
            prompt: "full".into(),
            prompt_delta: None,
            level_override: None,
            personal_level: None,
        };
        let outcome = run_host_turn(spec, |_req| {
            Err("chat turn in joyint-project-x: <err>connection reset by peer</err>".into())
        });
        assert!(outcome.reply.is_empty());
        assert_eq!(
            outcome.notice,
            "@vibe could not finish this turn: connection reset by peer"
        );
    }

    #[test]
    fn the_level_order_is_override_personal_member_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        // no project store at all: the project default is the floor
        let floor = host_turn_level(dir.path(), "ai:vibe@joy", None, None);
        assert_eq!(
            floor,
            joy_core::store::load_interaction_level_defaults(dir.path()).default
        );
        // personal beats the defaults, the chat override beats personal
        assert_eq!(
            host_turn_level(
                dir.path(),
                "ai:vibe@joy",
                None,
                Some(InteractionLevel::Autonomous)
            ),
            InteractionLevel::Autonomous
        );
        assert_eq!(
            host_turn_level(
                dir.path(),
                "ai:vibe@joy",
                Some(InteractionLevel::Proposing),
                Some(InteractionLevel::Autonomous)
            ),
            InteractionLevel::Proposing
        );
    }

    #[test]
    fn the_wire_shape_reuses_the_fields_the_clients_already_read() {
        let pending = WireActivity::pending("pending-x-1");
        assert_eq!(pending.kind, "pending");
        assert_eq!(pending.id, "pending-x-1");
        let tool = WireActivity::of(&TurnActivity::Tool {
            id: "t1".into(),
            title: "joy ls".into(),
            status: "completed".into(),
        });
        assert_eq!(
            (
                tool.kind.as_str(),
                tool.text.as_str(),
                tool.tool.as_str(),
                tool.payload.as_str()
            ),
            ("turn-tool", "joy ls", "t1", "completed")
        );
        let chunk = WireActivity::of(&TurnActivity::Chunk { text: "hi".into() });
        assert_eq!(
            (chunk.kind.as_str(), chunk.text.as_str()),
            ("turn-chunk", "hi")
        );
    }

    #[test]
    fn a_finished_turn_records_what_it_cost() {
        // JAPP-016A-E0: the popover lost tokens and cost when the turn
        // moved out of the engine loop and each host kept only the
        // activity block.
        let out = TurnOutcome {
            reply: "da".into(),
            tool_steps: 2,
            cost_cents: Some(7),
            tokens: Some(1234),
            model: Some("mistral-medium".into()),
            ..Default::default()
        };
        let details = turn_details(&out, InteractionLevel::Autonomous)
            .expect("a turn that reports something keeps a record");
        assert!(details.contains("1234"), "{details}");
        assert!(details.contains("mistral-medium"), "{details}");
        assert!(details.contains("autonomous"), "{details}");
        let bare = TurnOutcome {
            reply: "da".into(),
            ..Default::default()
        };
        assert!(turn_details(&bare, InteractionLevel::Proposing).is_some());
    }

    #[test]
    fn the_budget_notice_names_the_member_and_the_way_out() {
        let note = budget_notice("vibe");
        assert!(note.starts_with("@vibe "), "{note}");
        assert!(note.contains("Settings"), "{note}");
    }

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
        assert!(!msg.contains("joyint-project"), "leaked container: {msg}");
        assert!(
            !msg.contains('<') && !msg.contains('>'),
            "leaked tags: {msg}"
        );
        assert!(!msg.contains("$0.05"), "echoed the raw limit: {msg}");
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
