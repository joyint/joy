// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! What went over the wire between the lane and its agent (JP-0166-48).
//!
//! A turn that ends without an answer used to leave nothing behind: the
//! lane knew that the agent was silent, not what had been said before the
//! silence, and the agent's own last words on stderr went nowhere. This
//! record keeps every line of the three streams as ONE short fact each
//! (which request, which answer, which update, which stderr line) and
//! hands the facts of a turn to whoever has to explain its ending.
//!
//! No content travels here: a request is its method, an update its kind,
//! an error its code and message. A stderr line is the agent's own text
//! and is kept, cut, with anything that looks like a credential masked.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

use agent_client_protocol::LineDirection;

/// The facts the record keeps; older ones fall out.
const KEEP: usize = 400;
/// The facts of one turn a reader gets: its last ones.
const TRAIL: usize = 40;
/// The longest stderr line or error message kept.
const TEXT: usize = 300;

struct Fact {
    seq: u64,
    at: Instant,
    /// `>` sent to the agent, `<` received, `!` its stderr.
    way: char,
    what: String,
    /// The same fact in a row (a stream of message chunks) counts here
    /// instead of filling the record.
    times: u32,
}

#[derive(Default)]
struct Facts {
    next: u64,
    kept: VecDeque<Fact>,
}

pub(super) struct Wire {
    facts: Mutex<Facts>,
}

impl Wire {
    pub(super) fn new() -> Self {
        Self {
            facts: Mutex::new(Facts::default()),
        }
    }

    /// One line of one stream, as the agent protocol's process transport
    /// hands it over.
    pub(super) fn record(&self, line: &str, direction: LineDirection) {
        let (way, what) = match direction {
            LineDirection::Stdin => ('>', message(line)),
            LineDirection::Stdout => ('<', message(line)),
            LineDirection::Stderr => ('!', stderr(line)),
        };
        let Some(what) = what else { return };
        let mut facts = self.facts.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(last) = facts.kept.back_mut() {
            if last.way == way && last.what == what {
                last.times += 1;
                return;
            }
        }
        tracing::info!(target: "joy_ai::acp_wire", way = %way, what = %what, "acp wire");
        let seq = facts.next;
        facts.next += 1;
        facts.kept.push_back(Fact {
            seq,
            at: Instant::now(),
            way,
            what,
            times: 1,
        });
        if facts.kept.len() > KEEP {
            facts.kept.pop_front();
        }
    }

    /// Where the record stands; a turn takes this when it starts.
    pub(super) fn mark(&self) -> u64 {
        self.facts.lock().unwrap_or_else(|e| e.into_inner()).next
    }

    /// The facts since `mark`, the last ones when there are many, each
    /// with the seconds since the mark was taken at `since`.
    pub(super) fn trail(&self, mark: u64, since: Instant) -> String {
        let facts = self.facts.lock().unwrap_or_else(|e| e.into_inner());
        let of_turn: Vec<&Fact> = facts.kept.iter().filter(|f| f.seq >= mark).collect();
        let skipped = of_turn.len().saturating_sub(TRAIL);
        let mut out = String::new();
        if skipped > 0 {
            out.push_str(&format!("({skipped} earlier) "));
        }
        for (i, fact) in of_turn.iter().skip(skipped).enumerate() {
            if i > 0 {
                out.push_str(" | ");
            }
            let at = fact.at.saturating_duration_since(since).as_secs_f32();
            out.push_str(&format!("{at:.1}s {} {}", fact.way, fact.what));
            if fact.times > 1 {
                out.push_str(&format!(" x{}", fact.times));
            }
        }
        if out.is_empty() {
            out.push_str("nothing went over the wire");
        }
        out
    }
}

/// A JSON-RPC line as its one fact: the method of a request or
/// notification (an update with its kind), or the answer to a request id.
fn message(line: &str) -> Option<String> {
    let value: serde_json::Value = match serde_json::from_str(line.trim()) {
        Ok(value) => value,
        Err(_) if line.trim().is_empty() => return None,
        Err(_) => return Some(format!("not json: {}", cut(&masked(line)))),
    };
    let id = value
        .get("id")
        .filter(|id| !id.is_null())
        .map(|id| format!(" #{id}"))
        .unwrap_or_default();
    if let Some(method) = value.get("method").and_then(|m| m.as_str()) {
        let kind = value["params"]["update"]["sessionUpdate"]
            .as_str()
            .map(|kind| format!(" {kind}"))
            .unwrap_or_default();
        return Some(format!("{method}{kind}{id}"));
    }
    if let Some(error) = value.get("error") {
        let code = error.get("code").map(|c| c.to_string()).unwrap_or_default();
        let text = error.get("message").and_then(|m| m.as_str()).unwrap_or("");
        return Some(format!("error{id} {code} {}", cut(&masked(text))));
    }
    if let Some(result) = value.get("result") {
        let stop = result
            .get("stopReason")
            .and_then(|s| s.as_str())
            .map(|stop| format!(" {stop}"))
            .unwrap_or_default();
        return Some(format!("result{id}{stop}"));
    }
    Some(format!("unknown message{id}"))
}

/// A stderr line as its fact. The frames of a traceback (indented lines)
/// say where in the agent, not what happened, and are left out.
fn stderr(line: &str) -> Option<String> {
    if line.trim().is_empty() {
        return None;
    }
    let indented = line.starts_with("    ") || line.starts_with('\t');
    let frame = line.trim_start().starts_with("File \"") || line.trim_start().starts_with('^');
    if frame || (indented && !line.contains(": ")) {
        return None;
    }
    Some(cut(&masked(line.trim())))
}

/// Anything shaped like a credential (a long run of token characters)
/// keeps its first four characters. An agent prints what its provider
/// answered, and a provider may quote the key it was given.
fn masked(text: &str) -> String {
    let token = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        let mixed =
            run.chars().any(|c| c.is_ascii_digit()) && run.chars().any(|c| c.is_ascii_alphabetic());
        if run.chars().count() >= 24 && mixed {
            out.extend(run.chars().take(4));
            out.push('…');
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in text.chars() {
        if token(c) {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

fn cut(text: &str) -> String {
    if text.chars().count() <= TEXT {
        return text.to_string();
    }
    let mut out: String = text.chars().take(TEXT).collect();
    out.push('…');
    out
}

/// What a protocol error SAYS, for a person: the agent's own words.
///
/// The transport wraps the end of an agent process as an internal error
/// whose text is a JSON object with the place in the library that
/// spawned the task, and only then, under `data`, what happened ("No
/// such container", the provider's refusal). Printed as it stands, the
/// reason reached the person as a line of library path, cut before the
/// cause (JP-0166-48).
pub(super) fn said(error: &agent_client_protocol::Error) -> String {
    let words = match &error.data {
        Some(serde_json::Value::String(text)) => Some(text.clone()),
        Some(serde_json::Value::Object(fields)) => fields
            .get("data")
            .or_else(|| fields.get("message"))
            .or_else(|| fields.get("error"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        _ => None,
    };
    let words = words.unwrap_or_default();
    let words = words.trim();
    if words.is_empty() || words == error.message {
        return cut(&masked(error.message.trim()));
    }
    cut(&masked(&format!("{}: {words}", error.message.trim())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_its_method_its_update_kind_or_its_answer() {
        let fact = |line: &str| message(line).unwrap();
        assert_eq!(
            fact(
                r#"{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"prompt":[{"type":"text","text":"secret words"}]}}"#
            ),
            "session/prompt #3"
        );
        assert_eq!(
            fact(
                r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"hi"}}}}"#
            ),
            "session/update agent_message_chunk"
        );
        assert_eq!(
            fact(r#"{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}"#),
            "result #3 end_turn"
        );
        assert_eq!(
            fact(
                r#"{"jsonrpc":"2.0","id":3,"error":{"code":-31001,"message":"Rate limit exceeded for mistral"}}"#
            ),
            "error #3 -31001 Rate limit exceeded for mistral"
        );
        assert_eq!(message("  "), None);
    }

    #[test]
    fn a_protocol_error_says_what_happened_not_where_in_the_library() {
        let mut error: agent_client_protocol::Error =
            agent_client_protocol::schema::v1::ErrorCode::InternalError.into();
        error.data = Some(serde_json::json!({
            "spawned_at": "/usr/local/cargo/registry/src/x/agent-client-protocol-1.3.0/src/jsonrpc.rs:1524:39",
            "data": "Process exited with exit status: 1: Error response from daemon: No such container: joyint-project-1\n",
        }));
        let reason = said(&error);
        assert!(!reason.contains("jsonrpc.rs"), "{reason}");
        assert!(
            reason.ends_with("Error response from daemon: No such container: joyint-project-1"),
            "{reason}"
        );
        // an error with words of its own keeps them
        let mut plain: agent_client_protocol::Error =
            agent_client_protocol::schema::v1::ErrorCode::InternalError.into();
        plain.message = "Rate limit exceeded for mistral".into();
        plain.data = Some(serde_json::json!({ "provider": "mistral" }));
        assert_eq!(said(&plain), "Rate limit exceeded for mistral");
    }

    #[test]
    fn stderr_keeps_what_happened_and_no_credential() {
        assert_eq!(
            stderr("mistralai.SDKError: Status 429. Body: {\"message\": \"slow down\"}").as_deref(),
            Some("mistralai.SDKError: Status 429. Body: {\"message\": \"slow down\"}")
        );
        assert_eq!(stderr("  File \"/usr/lib/x.py\", line 3, in y"), None);
        assert_eq!(stderr("    await task"), None);
        assert_eq!(
            stderr("  status: 503 Service Unavailable").as_deref(),
            Some("status: 503 Service Unavailable")
        );
        let line = stderr("key mstrl0123456789abcdefghijklmnop refused").unwrap();
        assert_eq!(line, "key mstr… refused");
        // a long word is not a credential
        assert_eq!(
            masked("internationalizationalization"),
            "internationalizationalization"
        );
    }

    #[test]
    fn the_trail_of_a_turn_starts_at_its_mark_and_counts_repeats() {
        let wire = Wire::new();
        wire.record(r#"{"id":1,"method":"initialize"}"#, LineDirection::Stdin);
        let mark = wire.mark();
        let since = Instant::now();
        wire.record(
            r#"{"id":3,"method":"session/prompt"}"#,
            LineDirection::Stdin,
        );
        for _ in 0..3 {
            wire.record(
                r#"{"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk"}}}"#,
                LineDirection::Stdout,
            );
        }
        wire.record("ERROR:root:Background task failed", LineDirection::Stderr);
        let trail = wire.trail(mark, since);
        assert!(!trail.contains("initialize"), "{trail}");
        assert!(trail.contains("> session/prompt #3"), "{trail}");
        assert!(
            trail.contains("< session/update agent_message_chunk x3"),
            "{trail}"
        );
        assert!(
            trail.ends_with("! ERROR:root:Background task failed"),
            "{trail}"
        );
        assert_eq!(wire.trail(wire.mark(), since), "nothing went over the wire");
    }
}
