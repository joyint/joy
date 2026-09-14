// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The questions a chat turn puts to the person it runs for (JOY-028A-DC,
//! JP-0135-17).
//!
//! The shared policy (`joy_chat::model::permission`) knows Allow and
//! Deny. A Deny can be escalated: a host with a human attached asks, a
//! host without one rejects. This module is the asking half, and it lives
//! with the lane because the lane is the only place a chat permission is
//! decided:
//!
//! * a host that has the delegating person attaches a [`PresentPerson`]
//!   to its [`super::TurnRequest`]; a host without one attaches nothing
//!   and every Deny rejects as before (the desktop attaches the person at
//!   it, the platform none yet, see [`PresentPerson`]);
//! * the lane opens a question in the person's [`TurnGate`], puts it on
//!   the turn's live wire (`TurnActivity::Gate`) and awaits the answer
//!   until the turn is cancelled;
//! * the host delivers the answer through [`TurnGate::answer`] with the
//!   member IT resolved from its own session, never one the client names,
//!   so nobody can answer a question put to someone else;
//! * [`answer_from_person`] turns the choice into the ACP answer and the
//!   record word.
//!
//! An answer that finds no open question (already answered, timed out,
//! put to someone else) is ignored: the lane's settled gate event is the
//! truth, and a late answer never grants anything.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use agent_client_protocol::schema::v1::{
    PermissionOptionId, PermissionOptionKind, RequestPermissionRequest,
};
use joy_chat::model::permission::Decision;
use tokio::sync::oneshot;

use super::{pick_option, reject_option, wire_word, PermissionAnswer};
use crate::turn_engine::{GateOption, GateQuestion};

/// What the present person chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateChoice {
    /// One of the options the agent offered, by its option id.
    Selected(String),
    /// The person declined without picking an option.
    Declined,
}

/// The person a turn runs for, attached by a host that has one
/// (JP-0135-17). The desktop attaches the person at it, this device's
/// identity (JAPP-026C-DC); the platform attaches none until JAPP-02A0-38
/// adds it through this same field.
#[derive(Clone)]
pub struct PresentPerson {
    /// Where the person's answers arrive.
    pub gate: Arc<TurnGate>,
    /// The member the question is put to; only their answer counts.
    pub member: String,
}

/// The registry of open questions through which a present person answers.
/// One per host process is enough: questions are keyed by turn and id.
#[derive(Default)]
pub struct TurnGate {
    /// (turn id, gate id) -> the open question
    open: Mutex<HashMap<(String, u32), OpenEntry>>,
    seq: AtomicU32,
}

/// One open question: who may answer it and where the answer goes.
struct OpenEntry {
    member: String,
    tx: oneshot::Sender<GateChoice>,
}

/// An open question, held by the lane task that awaits it. Dropping it
/// (answered, timed out, or the task dropped with a dead lane) removes its
/// entry, so the registry never leaks.
pub struct OpenGate {
    gate: Arc<TurnGate>,
    turn_id: String,
    /// The id the question carries on the wire and the answer names.
    pub id: u32,
    /// Receives the person's choice; errors when the turn settled first.
    pub rx: oneshot::Receiver<GateChoice>,
}

impl Drop for OpenGate {
    fn drop(&mut self) {
        self.gate
            .entries()
            .remove(&(std::mem::take(&mut self.turn_id), self.id));
    }
}

impl TurnGate {
    fn entries(&self) -> MutexGuard<'_, HashMap<(String, u32), OpenEntry>> {
        self.open.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Open a question of the turn `turn_id`, put to `member`.
    pub fn open(self: &Arc<Self>, turn_id: &str, member: &str) -> OpenGate {
        let id = self.seq.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        let (tx, rx) = oneshot::channel();
        self.entries().insert(
            (turn_id.to_string(), id),
            OpenEntry {
                member: member.to_string(),
                tx,
            },
        );
        OpenGate {
            gate: self.clone(),
            turn_id: turn_id.to_string(),
            id,
            rx,
        }
    }

    /// Deliver `member`'s answer: `Some(option id)` picks an option, None
    /// declines. A question that is not open, or was put to another
    /// member, is left alone. Neither is an error: the settled gate event
    /// already told everyone how the question ended. Member and option
    /// ids are never logged.
    pub fn answer(&self, turn_id: &str, id: u32, member: &str, option_id: Option<String>) {
        let key = (turn_id.to_string(), id);
        let mut open = self.entries();
        let asked_this_member = match open.get(&key) {
            Some(entry) => entry.member == member,
            None => {
                tracing::debug!(gate = id, "a gate answer found no open question");
                return;
            }
        };
        if !asked_this_member {
            tracing::debug!(gate = id, "a gate answer from another member was ignored");
            return;
        }
        if let Some(entry) = open.remove(&key) {
            let choice = match option_id {
                Some(option) => GateChoice::Selected(option),
                None => GateChoice::Declined,
            };
            // the waiting task may have ended in the same instant; the
            // question is closed either way
            let _ = entry.tx.send(choice);
        }
    }

    /// The turn ended: every question still open for it ends unanswered
    /// (its waiter sees the dropped sender).
    pub fn settle_turn(&self, turn_id: &str) {
        self.entries().retain(|(turn, _), _| turn != turn_id);
    }
}

/// Does the request offer an option that would let the call run? Only
/// then is there anything to ask: a request without one can only be
/// refused.
pub(super) fn offers_allow(request: &RequestPermissionRequest) -> bool {
    pick_option(
        request,
        [
            PermissionOptionKind::AllowOnce,
            PermissionOptionKind::AllowAlways,
        ],
    )
    .is_some()
}

/// The question as it rides the wire: the call it opens and the options
/// the agent offered, kinds as their wire words.
pub(super) fn gate_question(
    id: u32,
    request: &RequestPermissionRequest,
    title: String,
) -> GateQuestion {
    GateQuestion {
        id,
        call: request.tool_call.tool_call_id.0.to_string(),
        title,
        options: request
            .options
            .iter()
            .map(|option| GateOption {
                id: option.option_id.0.to_string(),
                name: option.name.clone(),
                kind: wire_word(&option.kind),
            })
            .collect(),
        answered: None,
    }
}

/// The lane's answer once the person chose, or did not (`choice` None:
/// the turn was cancelled, the agent withdrew the request, or the turn
/// ended first).
///
/// * an offered allow option runs the call: "allowed (person)";
/// * an offered reject option refuses it: "denied (person)";
/// * a decline, or an option the request never offered, refuses with the
///   agent's reject option, the same pick as the policy's own deny, but
///   never an allow option (see [`refusal_option`]): "denied (person)";
/// * no answer is `Cancelled`, as ACP asks of a client that cancels while
///   a permission is pending: "denied (no answer)".
pub fn answer_from_person(
    request: &RequestPermissionRequest,
    title: String,
    choice: Option<GateChoice>,
) -> PermissionAnswer {
    let Some(choice) = choice else {
        return PermissionAnswer {
            selected: None,
            title,
            answered: "denied (no answer)",
            question: None,
            decision: Decision::Deny,
        };
    };
    let offered = match &choice {
        GateChoice::Selected(option_id) => request
            .options
            .iter()
            .find(|option| option.option_id.0.as_ref() == option_id.as_str()),
        GateChoice::Declined => None,
    };
    match offered.map(|option| (option, option.kind)) {
        Some((option, PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways)) => {
            PermissionAnswer {
                selected: Some(option.option_id.clone()),
                title,
                answered: "allowed (person)",
                question: None,
                decision: Decision::Allow,
            }
        }
        Some((option, PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways)) => {
            PermissionAnswer {
                selected: Some(option.option_id.clone()),
                title,
                answered: "denied (person)",
                question: None,
                decision: Decision::Deny,
            }
        }
        // declined, not offered, or a kind this client does not know:
        // never a grant
        _ => PermissionAnswer {
            selected: refusal_option(request),
            title,
            answered: "denied (person)",
            question: None,
            decision: Decision::Deny,
        },
    }
}

/// The option a person's refusal selects: the lane's reject pick, but
/// never an option that would let the call run. `reject_option` falls
/// back to the agent's first option, and a request is only asked when it
/// offers an allow option, so on an agent without a reject option that
/// first option can be an allow option. The refusal then answers
/// Cancelled (None), so the call does not run and the record's
/// "denied (person)" stays true.
fn refusal_option(request: &RequestPermissionRequest) -> Option<PermissionOptionId> {
    reject_option(request).filter(|chosen| {
        request
            .options
            .iter()
            .find(|option| &option.option_id == chosen)
            .is_some_and(|option| {
                !matches!(
                    option.kind,
                    PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
                )
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp_lane::{answer_chat_permission, KnownCall};
    use tokio::sync::oneshot::error::TryRecvError;

    fn permission_request(value: serde_json::Value) -> RequestPermissionRequest {
        serde_json::from_value(value).expect("a valid permission request")
    }

    /// A mutating call offering the usual four options.
    fn delete_request() -> RequestPermissionRequest {
        permission_request(serde_json::json!({
            "sessionId": "s1",
            "toolCall": { "toolCallId": "t1", "title": "rm -rf target", "kind": "delete" },
            "options": [
                { "optionId": "once", "name": "Allow once", "kind": "allow_once" },
                { "optionId": "always", "name": "Always allow", "kind": "allow_always" },
                { "optionId": "no", "name": "Reject", "kind": "reject_once" },
                { "optionId": "never", "name": "Never", "kind": "reject_always" },
            ],
        }))
    }

    fn selected(answer: &PermissionAnswer) -> Option<&str> {
        answer.selected.as_ref().map(|option| option.0.as_ref())
    }

    #[test]
    fn a_persons_allow_selects_their_option_and_is_recorded_as_allowed_by_person() {
        let answer = answer_from_person(
            &delete_request(),
            "rm -rf target".into(),
            Some(GateChoice::Selected("always".into())),
        );
        assert_eq!(selected(&answer), Some("always"));
        assert_eq!(answer.answered, "allowed (person)");
        assert_eq!(answer.decision, Decision::Allow);
    }

    #[test]
    fn a_persons_reject_option_is_recorded_as_denied_by_person() {
        let answer = answer_from_person(
            &delete_request(),
            "rm -rf target".into(),
            Some(GateChoice::Selected("never".into())),
        );
        assert_eq!(selected(&answer), Some("never"));
        assert_eq!(answer.answered, "denied (person)");
        assert_eq!(answer.decision, Decision::Deny);
    }

    #[test]
    fn a_decline_picks_the_reject_option_never_cancelled() {
        let answer = answer_from_person(
            &delete_request(),
            "rm -rf target".into(),
            Some(GateChoice::Declined),
        );
        // the agent's own reject option, not Cancelled (the old desktop
        // path answered a decline Cancelled)
        assert_eq!(selected(&answer), Some("no"));
        assert_eq!(answer.answered, "denied (person)");
        assert_eq!(answer.decision, Decision::Deny);
    }

    #[test]
    fn an_option_the_request_did_not_offer_counts_as_a_decline() {
        let answer = answer_from_person(
            &delete_request(),
            "rm -rf target".into(),
            Some(GateChoice::Selected("made-up".into())),
        );
        assert_eq!(selected(&answer), Some("no"));
        assert_eq!(answer.answered, "denied (person)");
        assert_eq!(answer.decision, Decision::Deny);
    }

    #[test]
    fn a_decline_without_a_reject_option_never_selects_an_allow_option() {
        // an agent that offers only allow options: the lane still asks,
        // because the person could approve
        let allow_only = permission_request(serde_json::json!({
            "sessionId": "s1",
            "toolCall": { "toolCallId": "t4", "title": "git push", "kind": "execute" },
            "options": [
                { "optionId": "once", "name": "Allow once", "kind": "allow_once" },
                { "optionId": "always", "name": "Always allow", "kind": "allow_always" },
            ],
        }));
        assert!(offers_allow(&allow_only));
        for choice in [GateChoice::Declined, GateChoice::Selected("made-up".into())] {
            let answer = answer_from_person(&allow_only, "git push".into(), Some(choice));
            // Cancelled, never the first (allow) option
            assert!(answer.selected.is_none(), "a refusal must not grant");
            assert_eq!(answer.answered, "denied (person)");
            assert_eq!(answer.decision, Decision::Deny);
        }
    }

    #[test]
    fn no_answer_is_cancelled_and_recorded_as_denied_no_answer() {
        let answer = answer_from_person(&delete_request(), "rm -rf target".into(), None);
        assert!(answer.selected.is_none(), "None answers ACP Cancelled");
        assert_eq!(answer.answered, "denied (no answer)");
        assert_eq!(answer.decision, Decision::Deny);
    }

    #[test]
    fn only_a_denied_request_with_an_allow_option_is_askable() {
        let request = delete_request();
        assert!(offers_allow(&request));
        // a mutating call at the proposing level is refused by the policy
        let denied = answer_chat_permission(
            joy_chat::model::AgentMode::Plan,
            &request,
            &KnownCall::default(),
        );
        assert_eq!(denied.decision, Decision::Deny);
        // joy is always allowed, so there is nothing to ask
        let joy = permission_request(serde_json::json!({
            "sessionId": "s1",
            "toolCall": { "toolCallId": "t2", "title": "joy ls", "kind": "execute" },
            "options": [
                { "optionId": "y", "name": "Allow", "kind": "allow_once" },
                { "optionId": "n", "name": "Reject", "kind": "reject_once" },
            ],
        }));
        let allowed = answer_chat_permission(
            joy_chat::model::AgentMode::Plan,
            &joy,
            &KnownCall::default(),
        );
        assert_eq!(allowed.decision, Decision::Allow);
        // a request that offers no allow option can only be refused
        let reject_only = permission_request(serde_json::json!({
            "sessionId": "s1",
            "toolCall": { "toolCallId": "t3", "title": "push" },
            "options": [ { "optionId": "n", "name": "Reject", "kind": "reject_once" } ],
        }));
        assert!(!offers_allow(&reject_only));
    }

    #[test]
    fn an_answer_reaches_its_open_gate_once() {
        let gate = Arc::new(TurnGate::default());
        let mut open = gate.open("turn-1", "human:person@joy.test");
        gate.answer(
            "turn-1",
            open.id,
            "human:person@joy.test",
            Some("once".into()),
        );
        // the second answer finds the question closed and changes nothing
        gate.answer("turn-1", open.id, "human:person@joy.test", None);
        assert_eq!(open.rx.try_recv(), Ok(GateChoice::Selected("once".into())));
        assert!(gate.entries().is_empty());
    }

    #[test]
    fn an_answer_from_another_member_leaves_the_question_open() {
        let gate = Arc::new(TurnGate::default());
        let mut open = gate.open("turn-1", "human:person@joy.test");
        gate.answer(
            "turn-1",
            open.id,
            "human:colleague@joy.test",
            Some("once".into()),
        );
        assert_eq!(open.rx.try_recv(), Err(TryRecvError::Empty));
        // the person it was put to can still answer
        gate.answer("turn-1", open.id, "human:person@joy.test", None);
        assert_eq!(open.rx.try_recv(), Ok(GateChoice::Declined));
    }

    #[test]
    fn an_answer_for_another_turn_or_a_dropped_gate_finds_nothing() {
        let gate = Arc::new(TurnGate::default());
        let mut open = gate.open("turn-1", "human:person@joy.test");
        gate.answer(
            "turn-2",
            open.id,
            "human:person@joy.test",
            Some("once".into()),
        );
        assert_eq!(open.rx.try_recv(), Err(TryRecvError::Empty));
        let id = open.id;
        drop(open);
        // nothing is open any more: the answer is ignored without a panic
        gate.answer("turn-1", id, "human:person@joy.test", Some("once".into()));
        assert!(gate.entries().is_empty());
    }

    #[test]
    fn a_dropped_open_gate_leaves_no_entry() {
        let gate = Arc::new(TurnGate::default());
        let first = gate.open("turn-1", "human:person@joy.test");
        let second = gate.open("turn-1", "human:person@joy.test");
        assert_ne!(first.id, second.id);
        assert_eq!(gate.entries().len(), 2);
        drop(first);
        assert_eq!(gate.entries().len(), 1);
        drop(second);
        assert!(gate.entries().is_empty());
    }

    #[test]
    fn settling_a_turn_ends_its_open_questions() {
        let gate = Arc::new(TurnGate::default());
        let mut ending = gate.open("turn-1", "human:person@joy.test");
        let mut other = gate.open("turn-2", "human:person@joy.test");
        gate.settle_turn("turn-1");
        assert_eq!(ending.rx.try_recv(), Err(TryRecvError::Closed));
        assert_eq!(other.rx.try_recv(), Err(TryRecvError::Empty));
        gate.answer(
            "turn-2",
            other.id,
            "human:person@joy.test",
            Some("once".into()),
        );
        assert_eq!(other.rx.try_recv(), Ok(GateChoice::Selected("once".into())));
    }

    #[test]
    fn a_gate_question_names_its_call_and_option_kinds() {
        let question = gate_question(3, &delete_request(), "rm -rf target".into());
        assert_eq!(question.id, 3);
        assert_eq!(question.call, "t1");
        assert_eq!(question.title, "rm -rf target");
        assert_eq!(question.answered, None);
        let kinds: Vec<&str> = question.options.iter().map(|o| o.kind.as_str()).collect();
        assert_eq!(
            kinds,
            ["allow_once", "allow_always", "reject_once", "reject_always"]
        );
        assert_eq!(question.options[2].id, "no");
        assert_eq!(question.options[2].name, "Reject");
    }
}
