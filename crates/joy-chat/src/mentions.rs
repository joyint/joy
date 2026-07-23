// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! @mention parsing for chats: which members a message addresses, by full
//! member ref or short alias. Chat-level (used by joy-chat storage and by
//! joy-ai's turn rules), so it lives here per ADR-043.

/// The short alias an AI is @mentioned by: `ai:claude@joy` -> `claude`.
pub fn alias(member_id: &str) -> &str {
    member_id
        .strip_prefix("ai:")
        .and_then(|rest| rest.split('@').next())
        .unwrap_or(member_id)
}

/// The raw @mention tokens of `text` (cleaned like [`mentions`]).
fn mention_tokens(text: &str) -> Vec<&str> {
    text.split(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == '!' || c == '?')
        .filter_map(|w| w.strip_prefix('@'))
        .map(|w| w.trim_end_matches(['.', ':', ')']))
        .filter(|w| !w.is_empty())
        .collect()
}

/// Member ids among `candidates` that `text` @mentions, by full ref or by
/// short alias.
pub fn mentions<'a>(text: &str, candidates: &'a [String]) -> Vec<&'a String> {
    let tokens = mention_tokens(text);
    candidates
        .iter()
        .filter(|candidate| {
            tokens
                .iter()
                .any(|t| *t == candidate.as_str() || *t == alias(candidate))
        })
        .collect()
}

/// The @mention tokens of `text` that match NOBODY in `candidates`
/// (JAPP-010D-B0: an unknown @name must answer with a visible error, not
/// silently do nothing). Matching mirrors [`mentions`]: full ref or alias.
pub fn unknown_mentions(text: &str, candidates: &[String]) -> Vec<String> {
    mention_tokens(text)
        .into_iter()
        .filter(|t| {
            !candidates
                .iter()
                .any(|candidate| *t == candidate.as_str() || *t == alias(candidate))
        })
        .map(str::to_string)
        .collect()
}
