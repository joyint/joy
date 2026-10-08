// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The id an AI member had before it was known by its name (JI-019D-46).
//!
//! An AI member is `claude`. It used to be written with a prefix and a
//! suffix around that name, and that older id still arrives from
//! outside: typed by a person, in a file, a chat, a token or a log
//! written back then. Whatever is read is cut down to the name right
//! where it is read, and from there on joy knows the name only.
//!
//! THE one place in joy that knows the older spelling. Nothing else
//! spells it out, tests it or builds it.

use std::borrow::Cow;
use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer};

const BEFORE: &str = "ai:";
const AFTER: &str = "@joy";

/// The event log's way to name who an AI member acted for, after its id.
const DELEGATED_BY: &str = " delegated-by:";

/// Whether `id` is an AI member's older id.
pub fn is_older(id: &str) -> bool {
    id.starts_with(BEFORE)
}

/// The name in an AI member's older id; anything else as it is.
pub fn name(id: &str) -> &str {
    id.strip_prefix(BEFORE)
        .map(|rest| rest.strip_suffix(AFTER).unwrap_or(rest))
        .unwrap_or(id)
}

/// A member as it was read, with an AI member's older id cut down to
/// its name. Takes the event log's `<member> delegated-by:<person>` too.
pub fn member(read: &str) -> Cow<'_, str> {
    if !is_older(read) {
        return Cow::Borrowed(read);
    }
    match read.split_once(DELEGATED_BY) {
        Some((ai, person)) => Cow::Owned(format!("{}{DELEGATED_BY}{person}", name(ai))),
        None => Cow::Borrowed(name(read)),
    }
}

/// [`member`], for a string that is kept.
pub fn member_owned(read: String) -> String {
    if is_older(&read) {
        member(&read).into_owned()
    } else {
        read
    }
}

/// The older id of the AI member called `name`: what a signature or a
/// key from before was made over. Only for holding such a one against
/// what it was made over, never to write anything.
pub fn spelled(name: &str) -> String {
    format!("{BEFORE}{name}{AFTER}")
}

/// Whether `word` is an AI member's older id and nothing else: what a
/// person may still type where a member is asked for.
pub fn is_typed(word: &str) -> bool {
    word.len() > BEFORE.len() + AFTER.len()
        && word.starts_with(BEFORE)
        && word.ends_with(AFTER)
        && !word.contains(char::is_whitespace)
}

/// Read a member (serde `deserialize_with`).
pub fn de_member<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    String::deserialize(deserializer).map(member_owned)
}

/// Read a member that may be absent (serde `deserialize_with`).
pub fn de_member_opt<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.map(member_owned))
}

/// Read a list of members (serde `deserialize_with`).
pub fn de_members<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    Ok(Vec::<String>::deserialize(deserializer)?
        .into_iter()
        .map(member_owned)
        .collect())
}

/// Read a map kept by member (serde `deserialize_with`). Where both
/// spellings of one AI member stand in it, the one under the name wins.
pub fn de_by_member<'de, D, V>(deserializer: D) -> Result<BTreeMap<String, V>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    Ok(by_member(BTreeMap::<String, V>::deserialize(deserializer)?))
}

/// A map kept by member, with every AI member under its name. Where
/// both spellings of one AI member stand in it, the one under the name
/// wins.
pub fn by_member<V>(read: BTreeMap<String, V>) -> BTreeMap<String, V> {
    if !read.keys().any(|key| is_older(key)) {
        return read;
    }
    let mut out = BTreeMap::new();
    let mut older = Vec::new();
    for (key, value) in read {
        if is_older(&key) {
            older.push((member_owned(key), value));
        } else {
            out.insert(key, value);
        }
    }
    for (key, value) in older {
        out.entry(key).or_insert(value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_older_id_is_cut_down_to_the_name() {
        assert_eq!(member("ai:claude@joy"), "claude");
        assert_eq!(member("claude"), "claude");
        assert_eq!(member("horst@joydev.com"), "horst@joydev.com");
        assert_eq!(member("m-1234"), "m-1234");
    }

    #[test]
    fn the_event_logs_compound_form_keeps_its_person() {
        assert_eq!(
            member("ai:claude@joy delegated-by:horst@joydev.com"),
            "claude delegated-by:horst@joydev.com"
        );
        assert_eq!(
            member("claude delegated-by:horst@joydev.com"),
            "claude delegated-by:horst@joydev.com"
        );
    }

    #[test]
    fn a_member_reference_never_holds_an_older_id() {
        use crate::MemberRef;
        assert_eq!(MemberRef::new("ai:claude@joy").id(), "claude");
        assert_eq!(MemberRef::from("ai:vibe@joy").id(), "vibe");
        let read: Vec<MemberRef> =
            serde_json::from_str(r#"["ai:vibe@joy","vibe","a@b.c"]"#).unwrap();
        let ids: Vec<&str> = read.iter().map(MemberRef::id).collect();
        assert_eq!(ids, ["vibe", "vibe", "a@b.c"]);
    }

    #[test]
    fn an_older_id_that_slipped_through_is_never_a_person() {
        assert!(crate::is_ai_member("ai:claude@joy"));
    }

    #[test]
    fn only_a_whole_word_counts_as_typed() {
        assert!(is_typed("ai:claude@joy"));
        assert!(is_typed("ai:copilot-chat@joy"));
        assert!(!is_typed("claude"));
        assert!(!is_typed("ai:@joy"));
        assert!(!is_typed("ask ai:claude@joy to look"));
    }

    #[test]
    fn the_older_id_of_a_name_reads_back_as_the_name() {
        assert_eq!(spelled("claude"), "ai:claude@joy");
        assert_eq!(name(&spelled("reviewer")), "reviewer");
    }

    #[test]
    fn a_map_holds_each_ai_member_once_and_the_name_wins() {
        let read: BTreeMap<String, u8> = [
            ("ai:vibe@joy".to_string(), 1),
            ("vibe".to_string(), 2),
            ("ai:claude@joy".to_string(), 3),
            ("horst@joydev.com".to_string(), 4),
        ]
        .into();
        let kept = by_member(read);
        assert_eq!(kept.len(), 3);
        assert_eq!(kept["vibe"], 2);
        assert_eq!(kept["claude"], 3);
        assert_eq!(kept["horst@joydev.com"], 4);
    }
}
