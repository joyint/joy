// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Short forms of ADR-027 ids (`ACRONYM-TYPE-XXXX-YY`), the rule jyn
//! settled in JOT-002F-4D and joy now shares (JOY-026B-E7): a list shows
//! the counter alone (`1`, `A1`, `110`), and only rows whose counter
//! collides within the list keep their suffix (`A1-EA`, `A1-7F`). Input
//! takes any of `1`, `0001`, `1-AB`, `MPS-CHAT-0001`, `MPS-CHAT-0001-AB`
//! (case-insensitive) and normalises it to the full counter form. No `#`
//! anywhere: PowerShell reads it as a comment (JISITE-0051-CA).

use std::collections::HashMap;

/// `(counter, suffix)` of a full id under `prefix` (`MPS-CHAT-`), or of a
/// bare `XXXX[-YY]`.
fn split(prefix: &str, full: &str) -> (String, Option<String>) {
    let rest = full
        .strip_prefix(prefix)
        .or_else(|| {
            full.get(..prefix.len())
                .filter(|p| p.eq_ignore_ascii_case(prefix))
                .map(|_| &full[prefix.len()..])
        })
        .unwrap_or(full);
    match rest.split_once('-') {
        Some((c, s)) => (c.to_ascii_uppercase(), Some(s.to_ascii_uppercase())),
        None => (rest.to_ascii_uppercase(), None),
    }
}

fn strip_zeros(counter: &str) -> String {
    let trimmed = counter.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".into()
    } else {
        trimmed.into()
    }
}

/// The short form of one id, without disambiguation.
pub fn short_id(prefix: &str, full: &str) -> String {
    strip_zeros(&split(prefix, full).0)
}

/// Short forms for a list, keeping the suffix only where counters
/// collide; one string per input, same order.
pub fn format_ids(prefix: &str, full_ids: &[&str]) -> Vec<String> {
    let parsed: Vec<(String, Option<String>)> = full_ids.iter().map(|f| split(prefix, f)).collect();
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for (counter, _) in &parsed {
        *seen.entry(counter.as_str()).or_insert(0) += 1;
    }
    parsed
        .iter()
        .map(|(counter, suffix)| {
            let short = strip_zeros(counter);
            match (seen.get(counter.as_str()).copied().unwrap_or(1) > 1, suffix) {
                (true, Some(s)) => format!("{short}-{s}"),
                _ => short,
            }
        })
        .collect()
}

/// What a person typed, as `(counter, suffix)` in the full form's spelling
/// (`0001`, `Some("AB")`), or None when it is not an id at all (a name).
pub fn parse_input(prefix: &str, raw: &str) -> Option<(String, Option<String>)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (counter, suffix) = split(prefix, trimmed);
    let counter_ok =
        !counter.is_empty() && counter.len() <= 4 && counter.chars().all(|c| c.is_ascii_hexdigit());
    let suffix_ok = suffix
        .as_deref()
        .is_none_or(|s| s.len() == 2 && s.chars().all(|c| c.is_ascii_hexdigit()));
    if !counter_ok || !suffix_ok {
        return None;
    }
    Some((format!("{counter:0>4}"), suffix))
}

/// Whether a full id matches what the person typed: same counter, and the
/// same suffix when one was typed.
pub fn matches(prefix: &str, full: &str, typed: &(String, Option<String>)) -> bool {
    let (counter, suffix) = split(prefix, full);
    counter == typed.0
        && typed
            .1
            .as_deref()
            .is_none_or(|s| Some(s) == suffix.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_forms_follow_jyn_without_the_hash() {
        assert_eq!(short_id("MPS-CHAT-", "MPS-CHAT-00A1-EA"), "A1");
        assert_eq!(short_id("MPS-CHAT-", "MPS-CHAT-0001-7F"), "1");
        assert_eq!(short_id("MPS-CHAT-", "MPS-CHAT-0110"), "110");
        assert_eq!(
            format_ids(
                "MPS-CHAT-",
                &[
                    "MPS-CHAT-0001-7F",
                    "MPS-CHAT-00A1-EA",
                    "MPS-CHAT-00A1-7F",
                    "MPS-CHAT-0110-B3"
                ]
            ),
            vec!["1", "A1-EA", "A1-7F", "110"]
        );
    }

    #[test]
    fn input_takes_every_spelling_and_leaves_names_alone() {
        let p = "MPS-CHAT-";
        for raw in ["1", "0001", "mps-chat-0001", "MPS-CHAT-0001"] {
            assert_eq!(parse_input(p, raw), Some(("0001".into(), None)), "{raw}");
        }
        assert_eq!(
            parse_input(p, "1-ab"),
            Some(("0001".into(), Some("AB".into())))
        );
        assert_eq!(
            parse_input(p, "MPS-CHAT-0001-AB"),
            Some(("0001".into(), Some("AB".into())))
        );
        assert_eq!(parse_input(p, "general"), None);
        assert_eq!(parse_input(p, "Webhooks"), None);
        assert_eq!(parse_input(p, "2902d005c323929221263104f8aac38a"), None);
        let typed = parse_input(p, "1").unwrap();
        assert!(matches(p, "MPS-CHAT-0001-AB", &typed));
        assert!(!matches(p, "MPS-CHAT-0002-AB", &typed));
        let exact = parse_input(p, "1-CD").unwrap();
        assert!(!matches(p, "MPS-CHAT-0001-AB", &exact));
    }
}
