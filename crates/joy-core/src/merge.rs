// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Three-way YAML merge for Joy item / milestone / project files.
//!
//! Always resolves to a valid YAML document; never emits Git conflict
//! markers. Intended to be wired in via `.gitattributes` so Git invokes
//! it during `merge` operations. See JOY-00B0.
//!
//! Resolution rules at every nesting level:
//! - If both sides equal the base, no change. If only one side changed
//!   away from base, that change is taken. If both sides changed
//!   differently, the side with the later top-level `updated` timestamp
//!   wins (last-write-wins, deterministic).
//! - Mappings are merged key-by-key, recursively.
//! - Sequences are 3-way set-merged using a stable identity per element:
//!   `comments` keyed by `(author, date)`, `assignees` by `member`, all
//!   other sequences by the element value itself (full-record equality
//!   for sub-mappings).
//!
//! The top-level `updated` field falls out of the same rule: the
//! tiebreaker side already has the later timestamp, so taking its value
//! is equivalent to `max(ours.updated, theirs.updated)`.

use serde_yaml_ng::{Mapping, Value};

use crate::error::JoyError;

/// Returns true if the given file content is a Joy encrypted blob (see
/// `crypt::FILTER_MAGIC`). Encrypted blobs must never go through YAML
/// field-level merge: their internal structure is opaque ciphertext.
pub fn is_joycrypt_blob(bytes: &[u8]) -> bool {
    bytes.starts_with(b"JOYCRYPT")
}

/// Three-way merge of a YAML document.
pub fn merge_yaml_doc(base: &str, ours: &str, theirs: &str) -> Result<String, JoyError> {
    let base_v = parse(base)?;
    let ours_v = parse(ours)?;
    let theirs_v = parse(theirs)?;

    let tiebreaker = pick_tiebreaker(&ours_v, &theirs_v);
    let merged = merge_value(&base_v, &ours_v, &theirs_v, tiebreaker, "");
    serde_yaml_ng::to_string(&merged).map_err(JoyError::from)
}

fn parse(s: &str) -> Result<Value, JoyError> {
    if s.trim().is_empty() {
        Ok(Value::Null)
    } else {
        serde_yaml_ng::from_str(s).map_err(JoyError::from)
    }
}

#[derive(Copy, Clone)]
enum Side {
    Ours,
    Theirs,
}

fn pick_tiebreaker(ours: &Value, theirs: &Value) -> Side {
    match (top_level_updated(ours), top_level_updated(theirs)) {
        (Some(o), Some(t)) => {
            if o >= t {
                Side::Ours
            } else {
                Side::Theirs
            }
        }
        (Some(_), None) => Side::Ours,
        (None, Some(_)) => Side::Theirs,
        (None, None) => Side::Theirs,
    }
}

fn top_level_updated(v: &Value) -> Option<&str> {
    if let Value::Mapping(m) = v {
        if let Some(Value::String(s)) = m.get(string_key("updated")) {
            return Some(s.as_str());
        }
    }
    None
}

fn merge_value(base: &Value, ours: &Value, theirs: &Value, tb: Side, path: &str) -> Value {
    if ours == theirs {
        return ours.clone();
    }
    if ours == base {
        return theirs.clone();
    }
    if theirs == base {
        return ours.clone();
    }
    match (ours, theirs) {
        (Value::Mapping(o), Value::Mapping(t)) => {
            let b = if let Value::Mapping(m) = base {
                Some(m)
            } else {
                None
            };
            Value::Mapping(merge_mapping(b, o, t, tb, path))
        }
        (Value::Sequence(o), Value::Sequence(t)) => {
            let b = if let Value::Sequence(s) = base {
                Some(s.as_slice())
            } else {
                None
            };
            Value::Sequence(merge_sequence(b, o, t, path))
        }
        _ => match tb {
            Side::Ours => ours.clone(),
            Side::Theirs => theirs.clone(),
        },
    }
}

fn merge_mapping(
    base: Option<&Mapping>,
    ours: &Mapping,
    theirs: &Mapping,
    tb: Side,
    path: &str,
) -> Mapping {
    let mut result = Mapping::new();

    // Walk ours-keys first to preserve their order.
    for (k, ov) in ours {
        let bv = base.and_then(|b| b.get(k));
        let tv = theirs.get(k);
        let child_path = sub_path(path, k);
        match (bv, tv) {
            (_, Some(tv)) => {
                let bv_owned = bv.cloned().unwrap_or(Value::Null);
                let merged = merge_value(&bv_owned, ov, tv, tb, &child_path);
                result.insert(k.clone(), merged);
            }
            (Some(bv_), None) => {
                // theirs removed this key.
                if ov == bv_ {
                    // ours unchanged from base; respect the removal.
                } else {
                    // ours modified vs base, theirs removed -> tiebreaker.
                    if matches!(tb, Side::Ours) {
                        result.insert(k.clone(), ov.clone());
                    }
                }
            }
            (None, None) => {
                // ours-only addition.
                result.insert(k.clone(), ov.clone());
            }
        }
    }

    // Append theirs-only keys.
    for (k, tv) in theirs {
        if ours.contains_key(k) {
            continue;
        }
        let bv = base.and_then(|b| b.get(k));
        match bv {
            None => {
                // theirs-only addition.
                result.insert(k.clone(), tv.clone());
            }
            Some(bv_) => {
                // ours removed this key.
                if tv == bv_ {
                    // theirs unchanged from base; respect the removal.
                } else if matches!(tb, Side::Theirs) {
                    // ours removed, theirs modified -> tiebreaker.
                    result.insert(k.clone(), tv.clone());
                }
            }
        }
    }

    result
}

fn merge_sequence(
    base: Option<&[Value]>,
    ours: &[Value],
    theirs: &[Value],
    path: &str,
) -> Vec<Value> {
    let id = |v: &Value| list_identity(path, v);

    let base_ids: Vec<Value> = base.map(|b| b.iter().map(id).collect()).unwrap_or_default();
    let ours_ids: Vec<Value> = ours.iter().map(id).collect();
    let theirs_ids: Vec<Value> = theirs.iter().map(id).collect();

    let in_set = |set: &[Value], k: &Value| set.iter().any(|x| x == k);

    let mut result = Vec::new();
    let mut seen: Vec<Value> = Vec::new();

    for item in ours {
        let k = id(item);
        if in_set(&seen, &k) {
            continue;
        }
        // If base had this and theirs removed it, drop unless ours edited.
        if in_set(&base_ids, &k) && !in_set(&theirs_ids, &k) {
            continue;
        }
        result.push(item.clone());
        seen.push(k);
    }
    for item in theirs {
        let k = id(item);
        if in_set(&seen, &k) {
            continue;
        }
        if in_set(&base_ids, &k) && !in_set(&ours_ids, &k) {
            continue;
        }
        result.push(item.clone());
        seen.push(k);
    }
    result
}

/// Identity for a sequence element, given the dotted path of the
/// containing sequence. Falls back to full-record equality when no
/// special key applies (matches naive set-union behaviour).
fn list_identity(path: &str, item: &Value) -> Value {
    let last = path.rsplit('.').next().unwrap_or(path);
    match (last, item) {
        ("comments", Value::Mapping(m)) => {
            let author = m.get(string_key("author")).cloned().unwrap_or(Value::Null);
            let date = m.get(string_key("date")).cloned().unwrap_or(Value::Null);
            Value::Sequence(vec![author, date])
        }
        ("assignees", Value::Mapping(m)) => {
            m.get(string_key("member")).cloned().unwrap_or(Value::Null)
        }
        // chat timelines (ADR JAPP-00C9): the message id IS the identity;
        // pre-channel messages fall back to (at, author, text)
        ("messages", Value::Mapping(m)) => match m.get(string_key("id")) {
            Some(id) => id.clone(),
            None => Value::Sequence(vec![
                m.get(string_key("at")).cloned().unwrap_or(Value::Null),
                m.get(string_key("author")).cloned().unwrap_or(Value::Null),
                m.get(string_key("text")).cloned().unwrap_or(Value::Null),
            ]),
        },
        _ => item.clone(),
    }
}

fn string_key(s: &str) -> Value {
    Value::String(s.to_string())
}

fn sub_path(parent: &str, key: &Value) -> String {
    let key_str = match key {
        Value::String(s) => s.as_str(),
        _ => return parent.to_string(),
    };
    if parent.is_empty() {
        key_str.to_string()
    } else {
        format!("{}.{}", parent, key_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merge(base: &str, ours: &str, theirs: &str) -> Value {
        let s = merge_yaml_doc(base, ours, theirs).unwrap();
        serde_yaml_ng::from_str(&s).unwrap()
    }

    fn yaml(s: &str) -> Value {
        serde_yaml_ng::from_str(s).unwrap()
    }

    #[test]
    fn duplicate_updated_field_is_resolved_to_max() {
        let base = "id: A\nupdated: 2026-04-22T08:00:00Z\nstatus: open\n";
        let ours = "id: A\nupdated: 2026-04-24T15:00:00Z\nstatus: open\n";
        let theirs = "id: A\nupdated: 2026-04-28T14:00:00Z\nstatus: open\n";
        let merged = merge(base, ours, theirs);
        let m = merged.as_mapping().unwrap();
        assert_eq!(
            m.get(string_key("updated")).unwrap().as_str().unwrap(),
            "2026-04-28T14:00:00Z"
        );
    }

    #[test]
    fn scalar_conflict_resolved_by_later_updated() {
        let base = "id: A\nupdated: 2026-01-01T00:00:00Z\npriority: medium\n";
        let ours = "id: A\nupdated: 2026-04-02T00:00:00Z\npriority: high\n";
        let theirs = "id: A\nupdated: 2026-04-01T00:00:00Z\npriority: low\n";
        let merged = merge(base, ours, theirs);
        let m = merged.as_mapping().unwrap();
        assert_eq!(
            m.get(string_key("priority")).unwrap().as_str().unwrap(),
            "high",
            "ours has the later updated, so its priority must win"
        );
    }

    #[test]
    fn one_sided_scalar_change_is_kept() {
        let base = "id: A\nupdated: 2026-01-01T00:00:00Z\nstatus: open\n";
        let ours = "id: A\nupdated: 2026-04-01T00:00:00Z\nstatus: in-progress\n";
        let theirs = "id: A\nupdated: 2026-01-01T00:00:00Z\nstatus: open\n";
        let merged = merge(base, ours, theirs);
        assert_eq!(
            merged
                .as_mapping()
                .unwrap()
                .get(string_key("status"))
                .unwrap()
                .as_str()
                .unwrap(),
            "in-progress"
        );
    }

    #[test]
    fn comments_added_on_both_sides_are_unioned() {
        let base = "id: A\nupdated: 2026-01-01T00:00:00Z\ncomments:\n  - author: alice\n    date: 2026-04-01T10:00:00Z\n    text: plan\n";
        let ours = "id: A\nupdated: 2026-04-02T00:00:00Z\ncomments:\n  - author: alice\n    date: 2026-04-01T10:00:00Z\n    text: plan\n  - author: bob\n    date: 2026-04-02T09:00:00Z\n    text: review\n";
        let theirs = "id: A\nupdated: 2026-04-03T00:00:00Z\ncomments:\n  - author: alice\n    date: 2026-04-01T10:00:00Z\n    text: plan\n  - author: carol\n    date: 2026-04-03T08:00:00Z\n    text: ship\n";
        let merged = merge(base, ours, theirs);
        let comments = merged
            .as_mapping()
            .unwrap()
            .get(string_key("comments"))
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(comments.len(), 3);
        let authors: Vec<_> = comments
            .iter()
            .filter_map(|c| c.as_mapping())
            .filter_map(|m| m.get(string_key("author")))
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(authors, vec!["alice", "bob", "carol"]);
    }

    #[test]
    fn comment_dedup_uses_author_date_not_text() {
        // Pathological: same (author, date) on both sides with different text.
        // Tiebreaker keeps one entry. We don't assert which text wins here.
        let base = "id: A\nupdated: 2026-01-01T00:00:00Z\ncomments: []\n";
        let ours = "id: A\nupdated: 2026-04-02T00:00:00Z\ncomments:\n  - author: alice\n    date: 2026-04-01T10:00:00Z\n    text: ours\n";
        let theirs = "id: A\nupdated: 2026-04-03T00:00:00Z\ncomments:\n  - author: alice\n    date: 2026-04-01T10:00:00Z\n    text: theirs\n";
        let merged = merge(base, ours, theirs);
        let comments = merged
            .as_mapping()
            .unwrap()
            .get(string_key("comments"))
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(comments.len(), 1, "same (author,date) -> single entry");
    }

    #[test]
    fn deps_and_tags_unioned() {
        let base = "id: A\nupdated: 2026-01-01T00:00:00Z\ndeps: [JOY-0001]\ntags: [a]\n";
        let ours =
            "id: A\nupdated: 2026-04-01T00:00:00Z\ndeps: [JOY-0001, JOY-0002]\ntags: [a, b]\n";
        let theirs =
            "id: A\nupdated: 2026-04-02T00:00:00Z\ndeps: [JOY-0001, JOY-0003]\ntags: [a, c]\n";
        let merged = merge(base, ours, theirs);
        let m = merged.as_mapping().unwrap();
        let deps: Vec<_> = m
            .get(string_key("deps"))
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(deps, vec!["JOY-0001", "JOY-0002", "JOY-0003"]);
        let tags: Vec<_> = m
            .get(string_key("tags"))
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(tags, vec!["a", "b", "c"]);
    }

    #[test]
    fn assignees_dedupe_by_member() {
        let base = "id: A\nupdated: 2026-01-01T00:00:00Z\nassignees: []\n";
        let ours = "id: A\nupdated: 2026-04-01T00:00:00Z\nassignees:\n  - member: alice\n    as: [implement]\n";
        let theirs = "id: A\nupdated: 2026-04-02T00:00:00Z\nassignees:\n  - member: alice\n    as: [implement]\n  - member: bob\n";
        let merged = merge(base, ours, theirs);
        let assignees = merged
            .as_mapping()
            .unwrap()
            .get(string_key("assignees"))
            .unwrap()
            .as_sequence()
            .unwrap();
        let members: Vec<_> = assignees
            .iter()
            .filter_map(|v| v.as_mapping())
            .filter_map(|m| m.get(string_key("member")))
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(members, vec!["alice", "bob"]);
    }

    #[test]
    fn item_added_on_one_side_and_removed_on_other_falls_through_3way() {
        // base has dep X. ours keeps it. theirs removes it.
        let base = "id: A\nupdated: 2026-01-01T00:00:00Z\ndeps: [X]\n";
        let ours = "id: A\nupdated: 2026-04-01T00:00:00Z\ndeps: [X]\n";
        let theirs = "id: A\nupdated: 2026-04-02T00:00:00Z\ndeps: []\n";
        let merged = merge(base, ours, theirs);
        let deps = merged
            .as_mapping()
            .unwrap()
            .get(string_key("deps"))
            .unwrap()
            .as_sequence()
            .unwrap();
        assert!(
            deps.is_empty(),
            "removed by theirs, untouched by ours -> drop"
        );
    }

    #[test]
    fn missing_field_added_on_one_side_only() {
        let base = "id: A\nupdated: 2026-01-01T00:00:00Z\n";
        let ours = "id: A\nupdated: 2026-04-01T00:00:00Z\nmilestone: M1\n";
        let theirs = "id: A\nupdated: 2026-04-02T00:00:00Z\n";
        let merged = merge(base, ours, theirs);
        assert_eq!(
            merged
                .as_mapping()
                .unwrap()
                .get(string_key("milestone"))
                .unwrap()
                .as_str()
                .unwrap(),
            "M1"
        );
    }

    #[test]
    fn nested_mapping_merges_recursively() {
        // Both sides change different keys of a nested mapping.
        let base = yaml("a:\n  x: 1\n  y: 2\nupdated: 2026-01-01T00:00:00Z\n");
        let ours = yaml("a:\n  x: 10\n  y: 2\nupdated: 2026-04-01T00:00:00Z\n");
        let theirs = yaml("a:\n  x: 1\n  y: 20\nupdated: 2026-04-02T00:00:00Z\n");
        let s = merge_yaml_doc(
            &serde_yaml_ng::to_string(&base).unwrap(),
            &serde_yaml_ng::to_string(&ours).unwrap(),
            &serde_yaml_ng::to_string(&theirs).unwrap(),
        )
        .unwrap();
        let merged: Value = serde_yaml_ng::from_str(&s).unwrap();
        let a = merged.as_mapping().unwrap().get(string_key("a")).unwrap();
        let am = a.as_mapping().unwrap();
        assert_eq!(am.get(string_key("x")).unwrap().as_i64().unwrap(), 10);
        assert_eq!(am.get(string_key("y")).unwrap().as_i64().unwrap(), 20);
    }

    #[test]
    fn detects_joycrypt_blob() {
        assert!(is_joycrypt_blob(b"JOYCRYPT\x01\x07default..."));
        assert!(!is_joycrypt_blob(b"id: A\nupdated: 2026-01-01T00:00:00Z\n"));
        assert!(!is_joycrypt_blob(b"JOY"));
        assert!(!is_joycrypt_blob(b""));
    }

    #[test]
    fn no_updated_anywhere_falls_back_to_theirs() {
        let base = "id: A\nstatus: open\n";
        let ours = "id: A\nstatus: in-progress\n";
        let theirs = "id: A\nstatus: review\n";
        let merged = merge(base, ours, theirs);
        assert_eq!(
            merged
                .as_mapping()
                .unwrap()
                .get(string_key("status"))
                .unwrap()
                .as_str()
                .unwrap(),
            "review"
        );
    }
}

#[cfg(test)]
mod delete_marks_tests {
    use super::*;

    #[test]
    fn deleted_for_marks_survive_a_conflicting_merge() {
        // the operator's fear (2026-07-05): a merge must never resurrect a
        // chat by dropping delete-for-me marks — even when BOTH sides
        // changed the file and the tiebreaker points at the other side
        let base =
            "id: c\ntitle: T\nupdated: 2026-07-05T10:00:00Z\nparticipants: []\nmessages: []\n";
        let ours = "id: c\ntitle: T\nupdated: 2026-07-05T10:00:01Z\ndeleted_for:\n- horst@example.com\nparticipants: []\nmessages: []\n";
        let theirs = "id: c\ntitle: T\nupdated: 2026-07-05T11:00:00Z\nparticipants: []\nmessages:\n- id: m9\n  at: 2026-07-05T11:00:00Z\n  author: geordi@example.org\n  text: newer over there\n";
        let merged = merge_yaml_doc(base, ours, theirs).unwrap();
        assert!(merged.contains("horst@example.com"), "mark lost: {merged}");
        assert!(
            merged.contains("newer over there"),
            "message lost: {merged}"
        );

        // and both sides' marks union when both deleted for themselves
        let ours2 = "id: c\ntitle: T\nupdated: 2026-07-05T10:00:01Z\ndeleted_for:\n- horst@example.com\nparticipants: []\nmessages: []\n";
        let theirs2 = "id: c\ntitle: T\nupdated: 2026-07-05T10:00:02Z\ndeleted_for:\n- geordi@example.org\nparticipants: []\nmessages: []\n";
        let merged2 = merge_yaml_doc(base, ours2, theirs2).unwrap();
        assert!(
            merged2.contains("horst@example.com"),
            "ours mark lost: {merged2}"
        );
        assert!(
            merged2.contains("geordi@example.org"),
            "theirs mark lost: {merged2}"
        );
    }
}
