// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The sealed chat event model (ADR JAPP-002A-30, whole-file encryption).
//!
//! A chat is persisted NOT as a mutable `meta.yaml` + message files, but
//! as an append-only set of immutable, content-addressed, individually
//! sealed EVENTS. This is forced by two hard constraints that collide:
//!
//! 1. The **whole file** must be encrypted, so no title, author,
//!    participant, mode or session may sit in plaintext ([[JAPP-002A-30]]).
//! 2. `refs/joy/chats` merges **keylessly and blindly** (the forge and any
//!    peer union sealed blobs without ever decrypting, see
//!    [`crate::chat_ref`]). A whole-encrypted `meta.yaml` could no longer
//!    be field-merged, so concurrent edits to title / participants / modes
//!    on two devices would clobber each other.
//!
//! The append-only event log resolves both: every mutation is one small
//! sealed blob, keyless merge is a pure union by content id, and a
//! read-time **fold** (this module) reduces the events into a [`Chat`]
//! with conflict-free CRDT semantics (LWW registers by sealed stamp,
//! LWW-element-sets, and message union by id). Nothing about who/what is
//! ever in plaintext, yet convergence needs no key.
//!
//! This module is the pure semantic core: [`ChatEvent`], the [`fold`]
//! reducer, and [`diff`] (the delta a save appends). Crypto/epoch/read
//! events layer on in [`crate::chat_crypt`]; storage in
//! [`crate::chat_ref`].

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::agent_mode::AgentMode;
use crate::model::chat::{Chat, ChatKind, ChatMessage};

/// A hybrid logical clock: `wall_ms << 16 | counter`. Sealed inside every
/// event, it is the TOTAL order for last-writer-wins, so the fold is
/// deterministic regardless of git merge order and needs no plaintext
/// tiebreaker (`chat.updated` never touches disk).
pub type Hlc = u64;

/// Process-local monotonic tiebreaker within a wall-millisecond.
static HLC_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh HLC for a write happening now.
pub fn now_hlc() -> Hlc {
    hlc_at(Utc::now())
}

/// The HLC encoding a specific instant (a message's `at`, a migration's
/// original timestamp), with a monotonic low-16 counter so two events in
/// the same millisecond still order.
pub fn hlc_at(at: DateTime<Utc>) -> Hlc {
    let ms = at.timestamp_millis().max(0) as u64;
    let counter = HLC_COUNTER.fetch_add(1, Ordering::Relaxed) & 0xffff;
    (ms << 16) | counter
}

/// The wall instant an HLC encodes (its millisecond component). Used to
/// recover a chat's `created` from its founding epoch event.
pub fn hlc_to_datetime(hlc: Hlc) -> DateTime<Utc> {
    DateTime::from_timestamp_millis((hlc >> 16) as i64).unwrap_or_else(Utc::now)
}

/// A last-writer-wins stamp: the HLC plus a stable per-writer tag so two
/// concurrent writes at the same HLC still resolve deterministically and
/// identically on every device. `writer` is an opaque tag (a hash of the
/// writer's verify key), sealed with the event.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Stamp {
    pub hlc: Hlc,
    pub writer: String,
}

impl Stamp {
    pub fn new(writer: impl Into<String>) -> Self {
        Self {
            hlc: now_hlc(),
            writer: writer.into(),
        }
    }

    pub fn at(hlc: Hlc, writer: impl Into<String>) -> Self {
        Self {
            hlc,
            writer: writer.into(),
        }
    }
}

/// One immutable fact about a chat. Every event is sealed as its own blob
/// (`crate::chat_crypt`); the fold below reduces a set of them into a
/// [`Chat`]. Serialized tag-per-variant so the format is self-describing
/// and forward-compatible (new variants an old reader can skip).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ev", rename_all = "kebab-case")]
pub enum ChatEvent {
    /// LWW register: the chat title (`None` clears it).
    Title { stamp: Stamp, value: Option<String> },
    /// LWW register: the chat subtitle.
    Subtitle { stamp: Stamp, value: Option<String> },
    /// LWW register: the chat kind (general / team / direct).
    Kind { stamp: Stamp, value: ChatKind },
    /// LWW register: the delete-for-all frozen flag.
    ReadOnly { stamp: Stamp, value: bool },
    /// LWW register: the creator (set once, but a register for merge).
    CreatedBy { stamp: Stamp, value: Option<String> },
    /// LWW-element-set entry for a participant. `present=true` adds (its
    /// stamp doubles as that member's JOINED marker); `false` removes.
    Participant {
        stamp: Stamp,
        member: String,
        present: bool,
    },
    /// LWW register for a per-(agent, delegator) permission mode.
    Mode {
        stamp: Stamp,
        agent: String,
        delegator: String,
        value: AgentMode,
    },
    /// LWW register for an AI member's ACP session id.
    Session {
        stamp: Stamp,
        member: String,
        value: String,
    },
    /// LWW-element-set entry for `deleted_for` (a member dismissed a
    /// delete-for-all copy). Only ever added.
    DeletedFor {
        stamp: Stamp,
        member: String,
        present: bool,
    },
    /// An immutable message. Unioned by message id (conflict-free); the
    /// carried `at` is its timeline clock.
    Message { msg: Box<ChatMessage> },
    /// A key-epoch node in the rotation DAG (sealed under its own CK, so
    /// the custodian and epoch holders can read it). `created` orders the
    /// tips; `parents` are the epochs it rotated from. Consumed by
    /// [`crate::chat_store`], not by [`fold`].
    Epoch {
        epoch_id: String,
        #[serde(default)]
        parents: Vec<String>,
        created: Hlc,
        #[serde(default)]
        reason: String,
    },
    /// Coverage hint: `epoch_id`'s CK was wrapped for `member` under
    /// `vk_hex`. The custodian folds these to decide rotation and
    /// key-change re-wraps ([`crate::chat_store`]); [`fold`] ignores it.
    Cover {
        epoch_id: String,
        member: String,
        vk_hex: String,
    },
    /// MAX register: `member`'s read watermark, the HLC up to which they
    /// have read. Merges by MAX so a later/higher watermark always wins and
    /// a stale concurrent write never regresses it (a read marker only ever
    /// moves forward). Advanced by `joy chat read` and the clients.
    Read {
        stamp: Stamp,
        member: String,
        upto: Hlc,
    },
}

/// The stable storage id of a message (its own id, or the deterministic
/// synthetic one for pre-channel messages) — the union key.
fn message_key(m: &ChatMessage) -> String {
    if m.id.is_empty() {
        m.synthetic_id()
    } else {
        m.id.clone()
    }
}

/// A total order over message VERSIONS of the same id: (serialized length,
/// serialization). The enriched follow-up copy (payload, attribution)
/// serializes longer than the bare append, so it wins deterministically on
/// every device regardless of merge order.
fn message_rank(m: &ChatMessage) -> (usize, String) {
    let s = serde_yaml_ng::to_string(m).unwrap_or_default();
    (s.len(), s)
}

/// Reduce a set of events into a [`Chat`]. Order-independent: the same
/// events in any order (any git merge outcome) fold to the same chat.
///
/// `id` and `created` are chat-identity facts fixed at creation and
/// supplied by the storage layer (the opaque dir name and the earliest
/// event clock); everything else is derived from the events.
pub fn fold(id: impl Into<String>, created: DateTime<Utc>, events: &[ChatEvent]) -> Chat {
    // Per-key LWW winners: keep the highest-stamp value seen.
    let mut title: Option<(Stamp, Option<String>)> = None;
    let mut subtitle: Option<(Stamp, Option<String>)> = None;
    let mut kind: Option<(Stamp, ChatKind)> = None;
    let mut read_only: Option<(Stamp, bool)> = None;
    let mut created_by: Option<(Stamp, Option<String>)> = None;
    let mut participants: BTreeMap<String, (Stamp, bool)> = BTreeMap::new();
    let mut modes: BTreeMap<(String, String), (Stamp, AgentMode)> = BTreeMap::new();
    let mut sessions: BTreeMap<String, (Stamp, String)> = BTreeMap::new();
    let mut deleted_for: BTreeMap<String, (Stamp, bool)> = BTreeMap::new();
    let mut messages: BTreeMap<String, ChatMessage> = BTreeMap::new();
    // Read watermarks merge by MAX, never by stamp: a marker only advances.
    let mut read_max: BTreeMap<String, Hlc> = BTreeMap::new();

    // Replace the register winner iff the new stamp is strictly greater.
    fn win<T: Clone>(slot: &mut Option<(Stamp, T)>, stamp: &Stamp, value: &T) {
        if slot.as_ref().is_none_or(|(s, _)| stamp > s) {
            *slot = Some((stamp.clone(), value.clone()));
        }
    }
    fn win_map<K: Ord + Clone, T: Clone>(
        map: &mut BTreeMap<K, (Stamp, T)>,
        key: K,
        stamp: &Stamp,
        value: &T,
    ) {
        match map.get(&key) {
            Some((s, _)) if s >= stamp => {}
            _ => {
                map.insert(key, (stamp.clone(), value.clone()));
            }
        }
    }

    for ev in events {
        match ev {
            ChatEvent::Title { stamp, value } => win(&mut title, stamp, value),
            ChatEvent::Subtitle { stamp, value } => win(&mut subtitle, stamp, value),
            ChatEvent::Kind { stamp, value } => win(&mut kind, stamp, value),
            ChatEvent::ReadOnly { stamp, value } => win(&mut read_only, stamp, value),
            ChatEvent::CreatedBy { stamp, value } => win(&mut created_by, stamp, value),
            ChatEvent::Participant {
                stamp,
                member,
                present,
            } => win_map(&mut participants, member.clone(), stamp, present),
            ChatEvent::Mode {
                stamp,
                agent,
                delegator,
                value,
            } => win_map(&mut modes, (agent.clone(), delegator.clone()), stamp, value),
            ChatEvent::Session {
                stamp,
                member,
                value,
            } => win_map(&mut sessions, member.clone(), stamp, value),
            ChatEvent::DeletedFor {
                stamp,
                member,
                present,
            } => win_map(&mut deleted_for, member.clone(), stamp, present),
            ChatEvent::Message { msg } => {
                // A message id can appear in more than one event: the bare
                // append first, then the ENRICHED copy (tool result payload,
                // AI attribution) from the follow-up save. Pick per id by a
                // total order — the larger canonical serialization wins (the
                // enriched copy carries more fields), tie-broken lexically —
                // so every merge order folds to the same chat.
                match messages.entry(message_key(msg)) {
                    std::collections::btree_map::Entry::Vacant(v) => {
                        v.insert((**msg).clone());
                    }
                    std::collections::btree_map::Entry::Occupied(mut o) => {
                        if message_rank(msg) > message_rank(o.get()) {
                            o.insert((**msg).clone());
                        }
                    }
                }
            }
            ChatEvent::Read { member, upto, .. } => {
                let slot = read_max.entry(member.clone()).or_insert(0);
                *slot = (*slot).max(*upto);
            }
            // crypto-plane events carry no semantic chat state.
            ChatEvent::Epoch { .. } | ChatEvent::Cover { .. } => {}
        }
    }

    let mut chat = Chat::new(id, Vec::new(), created);
    chat.title = title.and_then(|(_, v)| v);
    chat.subtitle = subtitle.and_then(|(_, v)| v);
    if let Some((_, k)) = kind {
        chat.kind = k;
    }
    chat.read_only = read_only.map(|(_, v)| v).unwrap_or(false);
    chat.created_by = created_by
        .and_then(|(_, v)| v)
        .map(crate::member_ref::MemberRef::new);
    chat.participants = participants
        .into_iter()
        .filter(|(_, (_, present))| *present)
        .map(|(m, _)| crate::member_ref::MemberRef::new(m))
        .collect();
    chat.modes = fold_modes(modes);
    chat.ai_sessions = sessions.into_iter().map(|(m, (_, v))| (m, v)).collect();
    chat.deleted_for = deleted_for
        .into_iter()
        .filter(|(_, (_, present))| *present)
        .map(|(m, _)| crate::member_ref::MemberRef::new(m))
        .collect();
    chat.read_markers = read_max
        .into_iter()
        .map(|(m, hlc)| (m, hlc_to_datetime(hlc)))
        .collect();

    // Messages in timeline order: by `at`, then by id for a stable tie.
    let mut msgs: Vec<ChatMessage> = messages.into_values().collect();
    msgs.sort_by(|a, b| {
        a.at.cmp(&b.at)
            .then_with(|| message_key(a).cmp(&message_key(b)))
    });
    chat.messages = msgs;

    // `updated` is an in-memory convenience (never serialized): the newest
    // activity by message clock, falling back to created.
    chat.updated = chat
        .messages
        .last()
        .map(|m| m.at)
        .unwrap_or(created)
        .max(created);
    chat
}

fn fold_modes(
    modes: BTreeMap<(String, String), (Stamp, AgentMode)>,
) -> BTreeMap<String, BTreeMap<String, AgentMode>> {
    let mut out: BTreeMap<String, BTreeMap<String, AgentMode>> = BTreeMap::new();
    for ((agent, delegator), (_, mode)) in modes {
        out.entry(agent).or_default().insert(delegator, mode);
    }
    out
}

/// Compute the events a save must APPEND to turn `base` (the folded
/// baseline the storage already holds) into `next`. Only genuine changes
/// produce events; an unchanged field yields nothing. `writer` tags the
/// stamps. New/changed registers and set-membership get a fresh HLC;
/// messages that are new (by id) are emitted verbatim.
pub fn diff(base: &Chat, next: &Chat, writer: &str) -> Vec<ChatEvent> {
    let mut out = Vec::new();
    let stamp = || Stamp::new(writer);

    if base.title != next.title {
        out.push(ChatEvent::Title {
            stamp: stamp(),
            value: next.title.clone(),
        });
    }
    if base.subtitle != next.subtitle {
        out.push(ChatEvent::Subtitle {
            stamp: stamp(),
            value: next.subtitle.clone(),
        });
    }
    if base.kind != next.kind {
        out.push(ChatEvent::Kind {
            stamp: stamp(),
            value: next.kind,
        });
    }
    if base.read_only != next.read_only {
        out.push(ChatEvent::ReadOnly {
            stamp: stamp(),
            value: next.read_only,
        });
    }
    let base_creator = base.created_by.as_ref().map(|m| m.id().to_string());
    let next_creator = next.created_by.as_ref().map(|m| m.id().to_string());
    if base_creator != next_creator {
        out.push(ChatEvent::CreatedBy {
            stamp: stamp(),
            value: next_creator,
        });
    }

    diff_set(
        &member_ids(&base.participants),
        &member_ids(&next.participants),
        writer,
        &mut out,
        |stamp, member, present| ChatEvent::Participant {
            stamp,
            member,
            present,
        },
    );
    diff_set(
        &member_ids(&base.deleted_for),
        &member_ids(&next.deleted_for),
        writer,
        &mut out,
        |stamp, member, present| ChatEvent::DeletedFor {
            stamp,
            member,
            present,
        },
    );

    // ai_sessions: LWW register per member.
    for (member, session) in &next.ai_sessions {
        if base.ai_sessions.get(member) != Some(session) {
            out.push(ChatEvent::Session {
                stamp: stamp(),
                member: member.clone(),
                value: session.clone(),
            });
        }
    }
    // modes: LWW register per (agent, delegator).
    for (agent, per) in &next.modes {
        for (delegator, mode) in per {
            let unchanged = base
                .modes
                .get(agent)
                .and_then(|p| p.get(delegator))
                .is_some_and(|m| m == mode);
            if !unchanged {
                out.push(ChatEvent::Mode {
                    stamp: stamp(),
                    agent: agent.clone(),
                    delegator: delegator.clone(),
                    value: *mode,
                });
            }
        }
    }

    // messages: emit each new message once — and a CHANGED copy of a known
    // id too (append_tool_result / append_ai_reply enrich the message in a
    // follow-up save; dropping that emitted the bare copy only and tool
    // results lost their payload). The fold picks per id by message_rank.
    let base_by_key: std::collections::BTreeMap<String, &ChatMessage> =
        base.messages.iter().map(|m| (message_key(m), m)).collect();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for m in &next.messages {
        let key = message_key(m);
        if !seen.insert(key.clone()) {
            continue;
        }
        let changed = match base_by_key.get(&key) {
            None => true,
            Some(b) => *b != m,
        };
        if changed {
            out.push(ChatEvent::Message {
                msg: Box::new(m.clone()),
            });
        }
    }

    // read markers: MAX register per member; emit only when a watermark
    // advances. Compare at millisecond granularity — the fold stores the
    // watermark's millisecond, so a re-save of an unchanged chat must not
    // emit a spurious Read (idempotency, see chat_store::save).
    for (member, at) in &next.read_markers {
        let advanced = base
            .read_markers
            .get(member)
            .is_none_or(|b| at.timestamp_millis() > b.timestamp_millis());
        if advanced {
            out.push(ChatEvent::Read {
                stamp: stamp(),
                member: member.clone(),
                upto: hlc_at(*at),
            });
        }
    }

    out
}

fn member_ids(members: &[crate::member_ref::MemberRef]) -> std::collections::BTreeSet<String> {
    members.iter().map(|m| m.id().to_string()).collect()
}

fn diff_set(
    base: &std::collections::BTreeSet<String>,
    next: &std::collections::BTreeSet<String>,
    writer: &str,
    out: &mut Vec<ChatEvent>,
    make: impl Fn(Stamp, String, bool) -> ChatEvent,
) {
    for m in next.difference(base) {
        out.push(make(Stamp::new(writer), m.clone(), true));
    }
    for m in base.difference(next) {
        out.push(make(Stamp::new(writer), m.clone(), false));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::member_ref::MemberRef;
    use crate::model::chat::MessageKind;

    fn ts(sec: u32) -> DateTime<Utc> {
        format!("2026-07-19T00:00:{sec:02}Z").parse().unwrap()
    }

    fn msg(id: &str, sec: u32, author: &str, text: &str) -> ChatMessage {
        ChatMessage {
            id: id.into(),
            at: ts(sec),
            author: MemberRef::new(author),
            text: text.into(),
            kind: MessageKind::Text,
            delegated_by: None,
            turn_ms: None,
            tool_steps: None,
            tool: None,
            payload: None,
            details: None,
            enc: None,
            epoch: None,
        }
    }

    /// A round-trip: build a chat, diff from empty, fold back, compare.
    #[test]
    fn diff_then_fold_reconstructs_the_chat() {
        let base = Chat::new("c1", Vec::new(), ts(0));
        let mut next = base.clone();
        next.title = Some("Standup".into());
        next.kind = ChatKind::Team;
        next.participants = vec![
            MemberRef::new("horst@example.com"),
            MemberRef::new("ai:vibe@joy"),
        ];
        next.ai_sessions
            .insert("ai:vibe@joy".into(), "acp-42".into());
        next.modes
            .entry("ai:vibe@joy".into())
            .or_default()
            .insert("horst@example.com".into(), AgentMode::AcceptEdits);
        next.messages.push(msg("m1", 1, "horst@example.com", "hi"));
        next.messages.push(msg("m2", 2, "ai:vibe@joy", "hello"));

        let events = diff(&base, &next, "w1");
        let folded = fold("c1", ts(0), &events);

        assert_eq!(folded.title.as_deref(), Some("Standup"));
        assert_eq!(folded.kind, ChatKind::Team);
        assert_eq!(folded.participants.len(), 2);
        assert!(folded.participants.iter().any(|p| p.id() == "ai:vibe@joy"));
        assert_eq!(
            folded.ai_sessions.get("ai:vibe@joy").map(String::as_str),
            Some("acp-42")
        );
        assert_eq!(
            folded.mode_override("ai:vibe@joy", "horst@example.com"),
            Some(AgentMode::AcceptEdits)
        );
        assert_eq!(folded.messages.len(), 2);
        assert_eq!(folded.messages[0].id, "m1");
        assert_eq!(folded.messages[1].id, "m2");
    }

    /// The fold is ORDER-INDEPENDENT: any permutation of the events (any
    /// merge outcome) yields the identical chat.
    #[test]
    fn fold_is_order_independent() {
        let base = Chat::new("c", Vec::new(), ts(0));
        let mut a = base.clone();
        a.title = Some("A".into());
        a.participants = vec![MemberRef::new("x@e")];
        a.messages.push(msg("m1", 1, "x@e", "one"));
        let mut events = diff(&base, &a, "w1");
        // a later title change + a second message
        let mut b = a.clone();
        b.title = Some("B".into());
        b.messages.push(msg("m2", 2, "x@e", "two"));
        events.extend(diff(&a, &b, "w1"));

        let forward = fold("c", ts(0), &events);
        let mut rev = events.clone();
        rev.reverse();
        let backward = fold("c", ts(0), &rev);
        assert_eq!(forward.title, backward.title);
        assert_eq!(forward.title.as_deref(), Some("B"), "latest stamp wins");
        assert_eq!(forward.messages.len(), backward.messages.len());
        assert_eq!(forward.messages.len(), 2);
    }

    /// Concurrent edits to DIFFERENT fields both survive (no clobber),
    /// and a message added on each side unions without loss — the exact
    /// thing whole-file encryption over meta.yaml would have broken.
    #[test]
    fn concurrent_field_edits_and_messages_both_survive() {
        let mut origin = Chat::new("c", vec![MemberRef::new("a@e")], ts(0));
        origin.title = Some("orig".into());
        let seed = diff(&Chat::new("c", Vec::new(), ts(0)), &origin, "w0");

        // device A: change the title, add m1
        let mut a = origin.clone();
        a.title = Some("from-A".into());
        a.messages.push(msg("m1", 1, "a@e", "A says"));
        let ev_a = diff(&origin, &a, "wA");

        // device B: change the subtitle, add m2 (concurrent, no A knowledge)
        let mut b = origin.clone();
        b.subtitle = Some("from-B".into());
        b.messages.push(msg("m2", 2, "a@e", "B says"));
        let ev_b = diff(&origin, &b, "wB");

        // keyless union of all events (what merge_refs does), folded once
        let mut all = seed.clone();
        all.extend(ev_a);
        all.extend(ev_b);
        let merged = fold("c", ts(0), &all);

        assert_eq!(merged.title.as_deref(), Some("from-A"), "A's title kept");
        assert_eq!(
            merged.subtitle.as_deref(),
            Some("from-B"),
            "B's subtitle kept"
        );
        assert_eq!(merged.messages.len(), 2, "no message lost");
    }

    /// Read watermarks fold by MAX: the highest wins regardless of order,
    /// and a stale lower watermark never regresses it.
    #[test]
    fn read_markers_fold_by_max_order_independent() {
        let events = vec![
            ChatEvent::Read {
                stamp: Stamp::at(hlc_at(ts(5)), "w"),
                member: "x@e".into(),
                upto: hlc_at(ts(5)),
            },
            ChatEvent::Read {
                stamp: Stamp::at(hlc_at(ts(3)), "w"),
                member: "x@e".into(),
                upto: hlc_at(ts(3)),
            },
            ChatEvent::Read {
                stamp: Stamp::at(hlc_at(ts(9)), "w"),
                member: "x@e".into(),
                upto: hlc_at(ts(9)),
            },
        ];
        let forward = fold("c", ts(0), &events);
        let mut rev = events.clone();
        rev.reverse();
        let backward = fold("c", ts(0), &rev);
        assert_eq!(
            forward.read_markers.get("x@e").map(|d| d.timestamp()),
            Some(ts(9).timestamp())
        );
        assert_eq!(forward.read_markers, backward.read_markers);
    }

    /// A save emits a Read only when the watermark advances; a re-save at
    /// the same (or lower) watermark emits none (idempotency).
    #[test]
    fn diff_emits_read_only_on_advance() {
        let base = Chat::new("c", vec![MemberRef::new("x@e")], ts(0));
        let mut a = base.clone();
        a.read_markers.insert("x@e".into(), ts(5));
        let ev = diff(&base, &a, "w1");
        assert_eq!(
            ev.iter()
                .filter(|e| matches!(e, ChatEvent::Read { .. }))
                .count(),
            1
        );
        // re-diff at the same watermark: no Read event
        let ev_same = diff(&a, &a, "w1");
        assert!(ev_same.iter().all(|e| !matches!(e, ChatEvent::Read { .. })));
        // a lower watermark does not emit (never regress)
        let mut lower = a.clone();
        lower.read_markers.insert("x@e".into(), ts(3));
        let ev_lower = diff(&a, &lower, "w1");
        assert!(ev_lower
            .iter()
            .all(|e| !matches!(e, ChatEvent::Read { .. })));
    }

    /// Leaving is an LWW-element-set removal that a later re-add can undo.
    #[test]
    fn participant_leave_and_rejoin_resolve_by_stamp() {
        let base = Chat::new("c", Vec::new(), ts(0));
        let mut joined = base.clone();
        joined.participants = vec![MemberRef::new("x@e")];
        let mut events = diff(&base, &joined, "w1");
        // leave
        let left = base.clone();
        events.extend(diff(&joined, &left, "w1"));
        assert!(fold("c", ts(0), &events).participants.is_empty());
        // rejoin (a strictly later stamp)
        events.extend(diff(&left, &joined, "w1"));
        assert_eq!(fold("c", ts(0), &events).participants.len(), 1);
    }
}
