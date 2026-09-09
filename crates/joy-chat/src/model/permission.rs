// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! THE permission policy for agent tool calls (JI-0179-4F step 2).
//!
//! Until 2026-07 this question was answered three times: the desktop's
//! streaming session had a per-mode match, the desktop's chat turn had a
//! kind allowlist, and the platform's container lanes had a third
//! expression. They disagreed, and the disagreements were bugs the
//! operator found one at a time: chat turns refused to READ (so an agent
//! could not answer), and refused `switch_mode` (so vibe stayed stranded
//! in its own plan mode, JAPP-0140-8B) — fixed on the desktop, still
//! broken on the platform until this module.
//!
//! One rule now, one place:
//!
//! * a `joy` invocation is ALWAYS allowed (operator 2026-07-21): it is
//!   the agents' governed item interface and enforces its own
//!   capability and mode rules;
//! * everything non-mutating is always allowed — reading, searching,
//!   thinking, fetching, and switching the agent's OWN session mode.
//!   This gate runs per tool call, so the agent's internal mode grants
//!   it no rights, and refusing a read only makes the agent unable to
//!   answer;
//! * mutations follow the interaction level's derived agent mode:
//!   autonomous allows them, accept-edits allows EDITS and denies the
//!   rest, plan denies them all;
//! * an unknown or absent tool kind counts as mutating, so a kind we do
//!   not know yet starts safe rather than permitted.
//!
//! A [`Decision::Deny`] is escalatable: a host with a human attached (the
//! desktop's streaming session) asks them; a host without one (chat
//! turns, container lanes) rejects. That difference is WHO can answer,
//! not what the rule is.

use crate::model::AgentMode;

/// What a tool call does to the world, derived from the ACP tool kind's
/// wire name (snake_case: "read", "edit", "delete", "move", "search",
/// "execute", "think", "fetch", "switch_mode", "other"). The mapping
/// lives HERE so no host classifies on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolAction {
    /// Reading, searching, thinking, fetching, switching the session mode.
    ReadOnly,
    /// Editing files: the thing accept-edits mode is named after.
    Edit,
    /// Deleting, moving, executing, and everything unknown.
    Mutating,
}

impl ToolAction {
    /// Classify an ACP tool kind by its wire name; `None` (the request
    /// carried no kind) and unknown names count as mutating.
    pub fn from_wire(kind: Option<&str>) -> Self {
        match kind {
            Some("read") | Some("search") | Some("think") | Some("fetch") | Some("switch_mode") => {
                ToolAction::ReadOnly
            }
            Some("edit") => ToolAction::Edit,
            _ => ToolAction::Mutating,
        }
    }
}

/// The policy's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Not allowed by the mode. A host with a human attached escalates
    /// (asks them); a host without one rejects.
    Deny,
}

/// The one permission decision for an agent tool call.
pub fn permission_decision(mode: AgentMode, action: ToolAction, is_joy: bool) -> Decision {
    if is_joy {
        return Decision::Allow;
    }
    match action {
        ToolAction::ReadOnly => Decision::Allow,
        ToolAction::Edit => match mode {
            AgentMode::Autonomous | AgentMode::AcceptEdits => Decision::Allow,
            AgentMode::Plan => Decision::Deny,
        },
        ToolAction::Mutating => match mode {
            AgentMode::Autonomous => Decision::Allow,
            AgentMode::AcceptEdits | AgentMode::Plan => Decision::Deny,
        },
    }
}

/// Whether a permission request is for a `joy` invocation. The request
/// itself may carry no command (vibe leaves it null, spec-legal), so
/// callers pass whatever they recovered: the tool call's title and its
/// raw input. Execute/bash tools carry the command under "command" (or
/// "cmd"), as a shell string or an argv array whose first item is the
/// program.
pub fn command_invokes_joy(title: Option<&str>, raw_input: Option<&serde_json::Value>) -> bool {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(v) = raw_input {
        for key in ["command", "cmd"] {
            match v.get(key) {
                Some(serde_json::Value::String(s)) => candidates.push(s.clone()),
                Some(serde_json::Value::Array(a)) => {
                    if let Some(serde_json::Value::String(p)) = a.first() {
                        candidates.push(p.clone());
                    }
                }
                _ => {}
            }
        }
        if let serde_json::Value::String(s) = v {
            candidates.push(s.clone());
        }
    }
    if let Some(t) = title {
        // Titles look like "bash: joy ls" or plain "joy ls".
        candidates.push(t.split_once(": ").map(|(_, r)| r).unwrap_or(t).to_string());
    }
    candidates.iter().any(|c| is_bare_joy_command(c))
}

/// A single, un-chained `joy` invocation: `argv0` basename is exactly `joy`
/// and there is no shell operator that could run another program. Substring
/// matching would be unsafe (`echo joy`, `rm -rf; joy`, `sh -c joy`), so we
/// match the program actually run and refuse chaining/substitution/redirect.
fn is_bare_joy_command(cmd: &str) -> bool {
    let cmd = cmd.trim();
    if !is_one_plain_command(cmd) {
        return false;
    }
    let argv0 = cmd.split_whitespace().next().unwrap_or("");
    let argv0 = argv0.trim_matches(['"', '\'']);
    let base = argv0.rsplit(['/', '\\']).next().unwrap_or(argv0);
    base == "joy"
}

/// Whether the string runs exactly ONE program, with nothing in it that
/// could start a second.
///
/// The scan honours quoting, and that is the whole point of it. A plain
/// substring test used to reject any command CONTAINING a newline, which
/// made `joy add --description "two\nlines"` stop being a joy invocation
/// (JOY-027E-D6):
/// `is_joy` went false, and below autonomous the turn was denied with no
/// human to ask. Which adapter it hit was accident, since the caller
/// offers several candidates and an argv array is represented by its
/// program name alone - so one AI could file a multi-line item and the
/// next could not.
///
/// A newline inside quotes is text. Everything that genuinely starts
/// another program stays disqualifying, and substitution disqualifies
/// inside double quotes too, where the shell still performs it; single
/// quotes make everything literal. An unterminated quote is a string we
/// cannot reason about, so it counts as not plain.
fn is_one_plain_command(cmd: &str) -> bool {
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    // None outside quotes, else the quote character we are inside of.
    let mut quote: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            // Single quotes: literal until the closing one.
            Some('\'') => {
                if c == '\'' {
                    quote = None;
                }
                i += 1;
            }
            // Double quotes: a backslash escapes the next character, and
            // the shell still substitutes in here.
            Some(_) => {
                if c == '\\' {
                    i += 2;
                    continue;
                }
                if c == '`' || (c == '$' && chars.get(i + 1) == Some(&'(')) {
                    return false;
                }
                if c == '"' {
                    quote = None;
                }
                i += 1;
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    i += 1;
                }
                // Escapes the next character, a line continuation included.
                '\\' => i += 2,
                ';' | '|' | '&' | '<' | '>' | '\n' | '`' => return false,
                '$' if chars.get(i + 1) == Some(&'(') => return false,
                _ => i += 1,
            },
        }
    }
    quote.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_is_allowed_in_every_mode() {
        // The chat-turn stranding class (JAPP-0140-8B): an agent that may
        // not read cannot answer, and refusing switch_mode strands it in
        // its own plan mode.
        for mode in [
            AgentMode::Plan,
            AgentMode::AcceptEdits,
            AgentMode::Autonomous,
        ] {
            for kind in ["read", "search", "think", "fetch", "switch_mode"] {
                assert_eq!(
                    permission_decision(mode, ToolAction::from_wire(Some(kind)), false),
                    Decision::Allow,
                    "{mode:?} {kind}"
                );
            }
        }
    }

    #[test]
    fn mutations_follow_the_mode() {
        use Decision::*;
        let edit = ToolAction::from_wire(Some("edit"));
        let exec = ToolAction::from_wire(Some("execute"));
        assert_eq!(permission_decision(AgentMode::Plan, edit, false), Deny);
        assert_eq!(
            permission_decision(AgentMode::AcceptEdits, edit, false),
            Allow
        );
        assert_eq!(
            permission_decision(AgentMode::Autonomous, edit, false),
            Allow
        );
        assert_eq!(permission_decision(AgentMode::Plan, exec, false), Deny);
        assert_eq!(
            permission_decision(AgentMode::AcceptEdits, exec, false),
            Deny
        );
        assert_eq!(
            permission_decision(AgentMode::Autonomous, exec, false),
            Allow
        );
    }

    #[test]
    fn an_unknown_kind_starts_safe() {
        for kind in [None, Some("brand_new_kind"), Some("delete"), Some("move")] {
            assert_eq!(
                ToolAction::from_wire(kind),
                ToolAction::Mutating,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn joy_is_never_blocked() {
        // operator 2026-07-21: joy is the governed interface; it enforces
        // its own capability and mode rules
        assert_eq!(
            permission_decision(AgentMode::Plan, ToolAction::Mutating, true),
            Decision::Allow
        );
    }

    #[test]
    fn joy_command_detection_from_raw_input_and_title() {
        use serde_json::json;
        let joy =
            |t: Option<&str>, r: Option<serde_json::Value>| command_invokes_joy(t, r.as_ref());
        // bash raw_input, the vibe shape.
        assert!(joy(
            Some("bash: joy ls"),
            Some(json!({"command": "joy ls"}))
        ));
        assert!(joy(
            None,
            Some(json!({"command": "joy add task \"fix bug\""}))
        ));
        assert!(joy(None, Some(json!({"command": "joy start JI-0001"}))));
        // argv array form.
        assert!(joy(None, Some(json!({"command": ["joy", "ls", "--tree"]}))));
        // absolute path to joy.
        assert!(joy(None, Some(json!({"command": "/usr/local/bin/joy ls"}))));
        // title-only fallback.
        assert!(joy(Some("joy show JI-0002"), None));

        // NOT joy: chaining, substitution, wrappers, substrings.
        assert!(!joy(None, Some(json!({"command": "rm -rf .joy; joy ls"}))));
        assert!(!joy(None, Some(json!({"command": "echo joy"}))));
        assert!(!joy(None, Some(json!({"command": "joy ls | rm -rf x"}))));
        assert!(!joy(None, Some(json!({"command": "sh -c 'joy ls'"}))));
        assert!(!joy(
            None,
            Some(json!({"command": "cat .joy/items/x.yaml"}))
        ));
        assert!(!joy(
            Some("bash: ls -la"),
            Some(json!({"command": "ls -la"}))
        ));
        assert!(!joy(None, None));
    }

    #[test]
    fn a_newline_inside_an_argument_is_text_not_chaining() {
        use serde_json::json;
        let joy =
            |t: Option<&str>, r: Option<serde_json::Value>| command_invokes_joy(t, r.as_ref());

        // The report (JOY-027E-D6): an item whose description has
        // real line breaks. Every one of these was refused below
        // autonomous, so an AI could file only single-line items, and the
        // refusal read as the AI declining.
        assert!(joy(
            None,
            Some(json!({"command": "joy add task \"T\" --description \"line one\nline two\""}))
        ));
        assert!(joy(
            None,
            Some(json!({"command": "joy comment JI-0001 \"first\n\n- second\n- third\""}))
        ));
        // Single quotes are literal all through.
        assert!(joy(
            None,
            Some(json!({"command": "joy add task 'a\nb; not a command'"}))
        ));
        // A line continuation is still ONE command.
        assert!(joy(
            None,
            Some(json!({"command": "joy add task \"T\" \\\n  --priority low"}))
        ));

        // ...and everything that really starts a second program still is
        // refused, quoted or not.
        assert!(!joy(
            None,
            Some(json!({"command": "joy add task \"T\"\nrm -rf /"}))
        ));
        // Substitution runs INSIDE double quotes, so it disqualifies there.
        assert!(!joy(
            None,
            Some(json!({"command": "joy add task \"$(rm -rf /)\""}))
        ));
        assert!(!joy(
            None,
            Some(json!({"command": "joy add task \"`rm -rf /`\""}))
        ));
        // An unterminated quote is not a string we can reason about.
        assert!(!joy(None, Some(json!({"command": "joy add task \"oops"}))));
    }
}
