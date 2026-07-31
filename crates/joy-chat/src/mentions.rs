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

/// The words of `text` in order, split the way mentions are written.
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == '!' || c == '?')
        .filter(|w| !w.is_empty())
}

/// A word's mention token, or `None` when the word is not a mention.
fn as_token(word: &str) -> Option<&str> {
    word.strip_prefix('@')
        .map(|w| w.trim_end_matches(['.', ':', ')']))
        .filter(|w| !w.is_empty())
}

/// The raw @mention tokens of `text` (cleaned like [`mentions`]).
fn mention_tokens(text: &str) -> Vec<&str> {
    words(text).filter_map(as_token).collect()
}

/// The mentions in the LEADING run of `text`: `@a @b what do you think?`
/// mentions a and b, `thanks, I asked @a` mentions nobody.
///
/// Addressing is a position, not a spelling (operator rule 2026-07-27,
/// JOY-0239-02): an @name at the START addresses that member, an @name
/// anywhere else merely REFERS to them, the way people write in any
/// messenger. Everything else in the message belongs to the sender of the
/// last message.
pub fn leading_mentions<'a>(text: &str, candidates: &'a [String]) -> Vec<&'a String> {
    let mut tokens: Vec<&str> = Vec::new();
    for word in words(text) {
        match as_token(word) {
            Some(token) => tokens.push(token),
            None => break,
        }
    }
    candidates
        .iter()
        .filter(|candidate| {
            tokens
                .iter()
                .any(|t| *t == candidate.as_str() || *t == alias(candidate))
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn members() -> Vec<String> {
        vec!["ai:claude@joy".to_string(), "ai:vibe@joy".to_string()]
    }

    #[test]
    fn a_leading_run_of_mentions_addresses_all_of_them() {
        let members = members();
        assert_eq!(
            leading_mentions("@claude @vibe what do you think?", &members),
            vec!["ai:claude@joy", "ai:vibe@joy"],
        );
        // punctuation and the full ref spell the same address
        assert_eq!(
            leading_mentions("@ai:vibe@joy, please look", &members),
            vec!["ai:vibe@joy"],
        );
    }

    #[test]
    fn a_mention_after_the_first_word_addresses_nobody() {
        let members = members();
        assert!(leading_mentions("thanks, I asked @claude", &members).is_empty());
        assert!(leading_mentions("look at @vibe's answer", &members).is_empty());
        // ... while `mentions` still SEES it: the two answer different
        // questions (who is referred to vs. who is addressed)
        assert_eq!(
            mentions("thanks, I asked @claude", &members),
            vec!["ai:claude@joy"],
        );
    }
}
