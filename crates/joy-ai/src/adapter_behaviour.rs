// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! What a tool does DIFFERENTLY from the protocol, and how the lane turns
//! it back into the one record (Horst 2026-09-10).
//!
//! The registry (`crate::adapters`) is data: one row per tool and no
//! behaviour. That held until the plan. A plan-mode turn's plan reaches
//! the room differently per tool, read from the sources:
//!
//! - vibe (mistral-vibe 2.19.1) writes the plan to a file under its home
//!   (`plans/<time>-<slug>.md`) through `write_file`/`edit` on an
//!   allowlist, so no permission request ever shows it, and sends no ACP
//!   plan update. Over ACP it has no exit_plan_mode; its reminder tells
//!   the model to present the plan in prose, which the model may or may
//!   not do (on integration it put the plan into its reasoning instead).
//!   Its replies may carry its own control tags (`<vibe_warning>`,
//!   `<vibe_stop_event>`, …), which its own TUI strips before showing.
//! - Claude (claude-agent-acp 0.76) hands the plan text over in the
//!   `ExitPlanMode` tool call: title "Approve Plan", kind `switch_mode`,
//!   the plan as the call's text content and under `plan` in its raw
//!   input. Its todo list is rendered as ACP `plan` updates.
//! - The protocol itself: `plan` updates carrying entries with a status.
//!
//! Only code per tool can normalise that; a table cannot read a file. So
//! there is ONE behaviour per tool, holding nothing but its deviations,
//! and the lane applies it once when a turn ends. Everything else stays
//! the shared lane, on both hosts, for every adapter.

use std::path::Path;

use crate::acp_lane::{Collected, KnownCall};

/// One entry of a protocol plan (ACP `plan` update). Status and priority
/// are the lowercase wire words (`pending`, `in_progress`, `completed`;
/// `high`, `medium`, `low`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanItem {
    pub content: String,
    pub status: String,
    pub priority: String,
}

/// What a behaviour may consult besides the collected turn.
pub struct TurnFacts<'a> {
    /// The session's working directory (the repo checkout).
    pub cwd: &'a Path,
}

/// A tool's deviations from the protocol. Every method has the
/// protocol's own behaviour as its default; a tool overrides only what
/// it does differently.
pub trait ToolBehaviour: Send + Sync {
    /// The plan this turn produced, as Markdown, wherever the tool kept
    /// it. None when the turn produced none.
    fn plan(&self, facts: &TurnFacts, state: &Collected) -> Option<String> {
        let _ = facts;
        protocol_plan(state)
    }

    /// The reply as the tool means it to be read: its own control tags
    /// removed, nothing else touched.
    fn clean_reply(&self, reply: String) -> String {
        reply
    }
}

/// The behaviour for a registry adapter id. An unknown or empty id (the
/// platform's mock, a tool without deviations) gets the protocol's own.
pub fn behaviour_for(adapter: &str) -> &'static dyn ToolBehaviour {
    match adapter {
        "vibe" => &Vibe,
        "claude" => &Claude,
        _ => &Protocol,
    }
}

/// Apply the tool's behaviour to a turn that has ended: the plan in the
/// one shape, the reply as the tool means it. Called once per turn by
/// the lane, for chat turns and job rounds alike.
pub fn finish_turn(behaviour: &dyn ToolBehaviour, cwd: &Path, state: &mut Collected) {
    let facts = TurnFacts { cwd };
    state.plan = behaviour
        .plan(&facts, state)
        .filter(|plan| !plan.trim().is_empty());
    let reply = std::mem::take(&mut state.reply);
    state.reply = behaviour.clean_reply(reply);
}

/// The protocol's plan entries as one Markdown checklist: the shape the
/// record and the live wire carry, the same for every tool.
pub fn plan_entries_markdown(entries: &[PlanItem]) -> String {
    entries
        .iter()
        .map(|entry| {
            let (mark, suffix) = match entry.status.as_str() {
                "completed" => ("x", ""),
                "in_progress" | "inprogress" => (" ", " _(in progress)_"),
                _ => (" ", ""),
            };
            format!("- [{mark}] {}{suffix}", entry.content)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn protocol_plan(state: &Collected) -> Option<String> {
    (!state.plan_entries.is_empty()).then(|| plan_entries_markdown(&state.plan_entries))
}

/// The raw input of a call as an object. vibe sends its arguments as a
/// JSON STRING inside `rawInput` (`model_dump_json()`), Claude's bridge
/// as an object; both are the same facts.
fn input_object(call: &KnownCall) -> Option<serde_json::Value> {
    match call.raw_input.as_ref()? {
        serde_json::Value::String(text) => serde_json::from_str(text).ok(),
        other => Some(other.clone()),
    }
}

// ---------------------------------------------------------------------------
// The protocol: no deviations.
// ---------------------------------------------------------------------------

/// A tool that does what the protocol says and nothing else.
pub struct Protocol;

impl ToolBehaviour for Protocol {}

// ---------------------------------------------------------------------------
// Claude: the plan rides the ExitPlanMode call.
// ---------------------------------------------------------------------------

/// claude-agent-acp: the plan is the content of the "Approve Plan" call
/// (kind `switch_mode`), also under `plan` in its raw input. The todo
/// list arrives as protocol plan updates and is the fallback.
pub struct Claude;

impl ToolBehaviour for Claude {
    fn plan(&self, _facts: &TurnFacts, state: &Collected) -> Option<String> {
        // the last hand-over wins: a plan the model revised after a
        // refusal is the plan the person sees
        let handed_over = state
            .calls_in_order()
            .into_iter()
            .rev()
            .find_map(claude_plan_of);
        handed_over.or_else(|| protocol_plan(state))
    }
}

fn claude_plan_of(call: &KnownCall) -> Option<String> {
    if let Some(plan) = input_object(call)
        .as_ref()
        .and_then(|input| input.get("plan"))
        .and_then(|plan| plan.as_str())
        .filter(|plan| !plan.trim().is_empty())
    {
        return Some(plan.to_string());
    }
    if call.kind.as_deref() == Some("switch_mode") {
        return call
            .content
            .iter()
            .find(|text| !text.trim().is_empty())
            .cloned();
    }
    None
}

// ---------------------------------------------------------------------------
// vibe: the plan is a file, the reply may carry control tags.
// ---------------------------------------------------------------------------

/// mistral-vibe: the plan lives in `<VIBE_HOME>/plans/<time>-<slug>.md`,
/// written and edited through the tool calls the lane already records.
/// The file is the source of truth (edits change it in place); the host
/// reads it at the path the tool named, which resolves unchanged here
/// (the platform mounts the state dir path-identically, the desktop runs
/// the tool on its own disk). A file the host cannot read falls back to
/// the last whole write the tool made.
pub struct Vibe;

/// The tags vibe wraps its own messages in (`vibe/core/utils/tags.py`,
/// KNOWN_TAGS). Its TUI shows the text inside and drops the tags; the
/// room does the same.
const VIBE_TAGS: [&str; 4] = [
    "vibe_warning",
    "vibe_stop_event",
    "tool_error",
    "user_cancellation",
];

impl ToolBehaviour for Vibe {
    fn plan(&self, _facts: &TurnFacts, state: &Collected) -> Option<String> {
        let mut path: Option<String> = None;
        let mut last_written: Option<String> = None;
        for call in state.calls_in_order() {
            let Some(input) = input_object(call) else {
                continue;
            };
            let Some(file) = input.get("file_path").and_then(|v| v.as_str()) else {
                continue;
            };
            if !is_plan_file(file) {
                continue;
            }
            path = Some(file.to_string());
            if let Some(content) = input.get("content").and_then(|v| v.as_str()) {
                last_written = Some(content.to_string());
            }
        }
        let path = path?;
        match std::fs::read_to_string(&path) {
            Ok(text) if !text.trim().is_empty() => Some(text),
            _ => last_written.filter(|text| !text.trim().is_empty()),
        }
    }

    fn clean_reply(&self, reply: String) -> String {
        strip_vibe_tags(&reply)
    }
}

/// A Markdown file directly inside a `plans` directory: vibe's plan
/// file, wherever its home is.
fn is_plan_file(file: &str) -> bool {
    let path = Path::new(file);
    let in_plans = path
        .parent()
        .and_then(|dir| dir.file_name())
        .is_some_and(|name| name == "plans");
    in_plans && path.extension().is_some_and(|ext| ext == "md")
}

/// Drop vibe's tag markers, keep what they wrapped, exactly like
/// `TaggedText.from_string` in vibe's own TUI.
fn strip_vibe_tags(text: &str) -> String {
    let mut out = text.to_string();
    for tag in VIBE_TAGS {
        out = out
            .replace(&format!("<{tag}>"), "")
            .replace(&format!("</{tag}>"), "");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A collected turn with these calls announced in this order.
    fn turn_with(calls: Vec<KnownCall>) -> Collected {
        let mut state = Collected::default();
        for (at, call) in calls.into_iter().enumerate() {
            let id = format!("call-{at}");
            state.tool_rows.insert(id.clone(), at);
            state.tools.push(crate::activity::ToolStep::new(
                call.title.clone().unwrap_or_default(),
                "completed",
            ));
            state.tool_calls.insert(id, call);
        }
        state
    }

    fn entries() -> Vec<PlanItem> {
        vec![
            PlanItem {
                content: "read the backlog".into(),
                status: "completed".into(),
                priority: "high".into(),
            },
            PlanItem {
                content: "write the tests".into(),
                status: "in_progress".into(),
                priority: "medium".into(),
            },
            PlanItem {
                content: "ship".into(),
                status: "pending".into(),
                priority: "low".into(),
            },
        ]
    }

    #[test]
    fn the_protocol_plan_is_one_checklist() {
        assert_eq!(
            plan_entries_markdown(&entries()),
            "- [x] read the backlog\n- [ ] write the tests _(in progress)_\n- [ ] ship"
        );
        let empty = Collected::default();
        assert_eq!(
            Protocol.plan(
                &TurnFacts {
                    cwd: Path::new(".")
                },
                &empty
            ),
            None
        );
        let state = Collected {
            plan_entries: entries(),
            ..Default::default()
        };
        assert!(Protocol
            .plan(
                &TurnFacts {
                    cwd: Path::new(".")
                },
                &state
            )
            .is_some_and(|plan| plan.starts_with("- [x] read")));
    }

    #[test]
    fn claude_hands_the_plan_over_in_the_approve_plan_call() {
        // the bridge's shape (claude-agent-acp 0.76): title "Approve
        // Plan", kind switch_mode, the plan as text content and in the
        // raw input under `plan`
        let state = turn_with(vec![
            KnownCall {
                title: Some("Read a".into()),
                raw_input: Some(serde_json::json!({"file_path": "/repo/a"})),
                kind: Some("read".into()),
                content: vec![],
            },
            KnownCall {
                title: Some("Approve Plan".into()),
                raw_input: Some(serde_json::json!({"plan": "## Plan\n\n1. do a\n2. do b"})),
                kind: Some("switch_mode".into()),
                content: vec!["## Plan\n\n1. do a\n2. do b".into()],
            },
        ]);
        let plan = Claude.plan(
            &TurnFacts {
                cwd: Path::new("."),
            },
            &state,
        );
        assert_eq!(plan.as_deref(), Some("## Plan\n\n1. do a\n2. do b"));
    }

    #[test]
    fn claude_takes_the_content_when_the_raw_input_carries_no_plan() {
        let state = turn_with(vec![KnownCall {
            title: Some("Approve Plan".into()),
            raw_input: None,
            kind: Some("switch_mode".into()),
            content: vec!["the plan text".into()],
        }]);
        let plan = Claude.plan(
            &TurnFacts {
                cwd: Path::new("."),
            },
            &state,
        );
        assert_eq!(plan.as_deref(), Some("the plan text"));
    }

    #[test]
    fn claude_falls_back_to_its_todo_list() {
        let mut state = turn_with(vec![]);
        state.plan_entries = entries();
        let plan = Claude.plan(
            &TurnFacts {
                cwd: Path::new("."),
            },
            &state,
        );
        assert!(plan.is_some_and(|p| p.contains("- [ ] ship")));
    }

    #[test]
    fn vibe_reads_the_plan_file_it_wrote_and_edited() {
        let home = tempfile::tempdir().expect("tempdir");
        let plans = home.path().join("plans");
        std::fs::create_dir_all(&plans).unwrap();
        let file = plans.join("1788989551-keen-brave-grove.md");
        // the tool wrote, then edited in place: the FILE is the truth
        std::fs::write(&file, "# Plan\n\n- edited step").unwrap();
        let file = file.to_string_lossy().to_string();
        // vibe's rawInput is a JSON STRING (model_dump_json)
        let write = serde_json::json!({"file_path": file, "content": "# Plan\n\n- first draft"});
        let edit = serde_json::json!({"file_path": file, "old_string": "first draft", "new_string": "edited step"});
        let state = turn_with(vec![
            KnownCall {
                title: Some("Writing plan".into()),
                raw_input: Some(serde_json::Value::String(write.to_string())),
                kind: Some("edit".into()),
                content: vec![],
            },
            KnownCall {
                title: Some("Editing plan".into()),
                raw_input: Some(serde_json::Value::String(edit.to_string())),
                kind: Some("edit".into()),
                content: vec![],
            },
        ]);
        let plan = Vibe.plan(&TurnFacts { cwd: home.path() }, &state);
        assert_eq!(plan.as_deref(), Some("# Plan\n\n- edited step"));
    }

    #[test]
    fn vibe_falls_back_to_the_last_whole_write_when_the_file_is_out_of_reach() {
        let state = turn_with(vec![KnownCall {
            title: Some("Writing plan".into()),
            raw_input: Some(serde_json::json!({
                "file_path": "/nowhere/on/this/host/plans/x.md",
                "content": "# Plan\n\n- as written"
            })),
            kind: Some("edit".into()),
            content: vec![],
        }]);
        let plan = Vibe.plan(
            &TurnFacts {
                cwd: Path::new("."),
            },
            &state,
        );
        assert_eq!(plan.as_deref(), Some("# Plan\n\n- as written"));
    }

    #[test]
    fn vibe_ignores_writes_outside_the_plans_dir() {
        let state = turn_with(vec![KnownCall {
            title: Some("Writing".into()),
            raw_input: Some(
                serde_json::json!({"file_path": "/repo/src/main.rs", "content": "fn main() {}"}),
            ),
            kind: Some("edit".into()),
            content: vec![],
        }]);
        assert_eq!(
            Vibe.plan(
                &TurnFacts {
                    cwd: Path::new(".")
                },
                &state
            ),
            None
        );
        assert!(!is_plan_file("/home/x/.vibe/plans/notes.txt"));
        assert!(!is_plan_file("/home/x/.vibe/plans/deeper/plan.md"));
        assert!(is_plan_file("/home/x/.vibe/plans/1-slug.md"));
    }

    #[test]
    fn vibe_control_tags_are_dropped_and_their_text_kept() {
        assert_eq!(
            strip_vibe_tags("<vibe_stop_event>Turn limit of 3 reached</vibe_stop_event>"),
            "Turn limit of 3 reached"
        );
        assert_eq!(
            strip_vibe_tags("plain <b>html</b> stays"),
            "plain <b>html</b> stays"
        );
    }

    #[test]
    fn finish_turn_normalises_plan_and_reply_in_place() {
        let mut state = turn_with(vec![]);
        state.plan_entries = entries();
        state.reply = "<vibe_warning>note</vibe_warning> done".into();
        finish_turn(&Vibe, Path::new("."), &mut state);
        assert_eq!(state.reply, "note done");
        // vibe wrote no plan file: nothing invented
        assert_eq!(state.plan, None);
        finish_turn(&Protocol, Path::new("."), &mut state);
        assert!(state.plan.is_some_and(|p| p.starts_with("- [x]")));
    }

    #[test]
    fn unknown_tools_get_the_protocol() {
        let state = Collected {
            plan_entries: entries(),
            ..Default::default()
        };
        let facts = TurnFacts {
            cwd: Path::new("."),
        };
        assert!(behaviour_for("acp-mock").plan(&facts, &state).is_some());
        assert!(behaviour_for("").plan(&facts, &state).is_some());
    }
}
