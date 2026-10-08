// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Reading the name of an AI member (JI-019D-46): THE one way, for
//! every crate and every host.
//!
//! An AI member is its name: `claude`. Before that it was written in a
//! legacy form, with a prefix and a suffix around the name. Nothing joy
//! writes carries that form any more, and no code works with it. It is
//! accepted in two places only, and cut down to the name right there:
//!
//! - where something is READ that was written back then: a file, a
//!   chat, a token, a log, a local store;
//! - where a person TYPES it: an argument, a mention.
//!
//! Whoever reads a member from one of those calls [`read`] (or names one
//! of the `de` functions on a serde field) and holds the name from then
//! on. That is the whole method. Old data is not rewritten for it: it is
//! read this way for as long as it exists, and whatever is written is
//! written with the name.
//!
//! This is a migration applied on read, like the others
//! (`joy_core::migrations`). It lives in this crate because the chat
//! crate reads members too and has to compile for the browser, where
//! joy-core cannot go; joy-core hands it on as
//! `joy_core::migrations::ai_member_name`.
//!
//! THE one place that knows the legacy form. Nothing else spells it
//! out, tests for it or builds it; a test reads the sources of each
//! repository and fails where one does.

use std::borrow::Cow;
use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer};

const BEFORE: &str = "ai:";
const AFTER: &str = "@joy";

/// The event log's way to name who an AI member acted for, after its id.
const DELEGATED_BY: &str = " delegated-by:";

/// Whether `id` is in the legacy form.
pub fn is_legacy(id: &str) -> bool {
    id.starts_with(BEFORE)
}

/// The name in a bare id in the legacy form; anything else as it is.
fn bare(id: &str) -> &str {
    id.strip_prefix(BEFORE)
        .map(|rest| rest.strip_suffix(AFTER).unwrap_or(rest))
        .unwrap_or(id)
}

/// A member as it was read or typed: an AI member's name, whichever
/// form it came in; a person's id as it is. Takes the event log's
/// `<member> delegated-by:<person>` too.
pub fn read(read: &str) -> Cow<'_, str> {
    if !is_legacy(read) {
        return Cow::Borrowed(read);
    }
    match read.split_once(DELEGATED_BY) {
        Some((ai, person)) => Cow::Owned(format!("{}{DELEGATED_BY}{person}", bare(ai))),
        None => Cow::Borrowed(bare(read)),
    }
}

/// [`read`], for a string that is kept.
pub fn read_owned(read: String) -> String {
    if is_legacy(&read) {
        self::read(&read).into_owned()
    } else {
        read
    }
}

/// The legacy form of the AI member called `name`: what a signature or
/// a key from back then was made over. Only for holding such a one
/// against what it was made over, never to write anything.
pub fn legacy_form(name: &str) -> String {
    format!("{BEFORE}{name}{AFTER}")
}

/// Whether `word` is the legacy form of an AI member and nothing else:
/// what a person may still type where a member is asked for.
pub fn is_typed(word: &str) -> bool {
    word.len() > BEFORE.len() + AFTER.len()
        && word.starts_with(BEFORE)
        && word.ends_with(AFTER)
        && !word.contains(char::is_whitespace)
}

/// Read a member (serde `deserialize_with`).
pub fn de<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    String::deserialize(deserializer).map(read_owned)
}

/// Read a member that may be absent (serde `deserialize_with`).
pub fn de_opt<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.map(read_owned))
}

/// Read a list of members (serde `deserialize_with`).
pub fn de_list<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    Ok(Vec::<String>::deserialize(deserializer)?
        .into_iter()
        .map(read_owned)
        .collect())
}

/// Read a map kept by member (serde `deserialize_with`), see
/// [`read_map`].
pub fn de_map<'de, D, V>(deserializer: D) -> Result<BTreeMap<String, V>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    Ok(read_map(BTreeMap::<String, V>::deserialize(deserializer)?))
}

/// A map kept by member as it was read, with every AI member under its
/// name. Where one AI member stands in it in both forms, the entry
/// under the name wins.
pub fn read_map<V>(read: BTreeMap<String, V>) -> BTreeMap<String, V> {
    if !read.keys().any(|key| is_legacy(key)) {
        return read;
    }
    let mut out = BTreeMap::new();
    let mut older = Vec::new();
    for (key, value) in read {
        if is_legacy(&key) {
            older.push((read_owned(key), value));
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
    fn the_legacy_form_is_cut_down_to_the_name() {
        assert_eq!(read("ai:claude@joy"), "claude");
        assert_eq!(read("claude"), "claude");
        assert_eq!(read("horst@joydev.com"), "horst@joydev.com");
        assert_eq!(read("m-1234"), "m-1234");
    }

    #[test]
    fn the_event_logs_compound_form_keeps_its_person() {
        assert_eq!(
            read("ai:claude@joy delegated-by:horst@joydev.com"),
            "claude delegated-by:horst@joydev.com"
        );
        assert_eq!(
            read("claude delegated-by:horst@joydev.com"),
            "claude delegated-by:horst@joydev.com"
        );
    }

    #[test]
    fn a_member_reference_never_holds_the_legacy_form() {
        use crate::MemberRef;
        assert_eq!(MemberRef::new("ai:claude@joy").id(), "claude");
        assert_eq!(MemberRef::from("ai:vibe@joy").id(), "vibe");
        let read: Vec<MemberRef> =
            serde_json::from_str(r#"["ai:vibe@joy","vibe","a@b.c"]"#).unwrap();
        let ids: Vec<&str> = read.iter().map(MemberRef::id).collect();
        assert_eq!(ids, ["vibe", "vibe", "a@b.c"]);
    }

    #[test]
    fn a_legacy_form_that_slipped_through_is_never_a_person() {
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
    fn the_legacy_form_of_a_name_reads_back_as_the_name() {
        assert_eq!(legacy_form("claude"), "ai:claude@joy");
        assert_eq!(read(&legacy_form("reviewer")), "reviewer");
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
        let kept = read_map(read);
        assert_eq!(kept.len(), 3);
        assert_eq!(kept["vibe"], 2);
        assert_eq!(kept["claude"], 3);
        assert_eq!(kept["horst@joydev.com"], 4);
    }
}
