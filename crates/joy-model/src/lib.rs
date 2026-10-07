// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The few types that everything in Joy shares, kept in a crate that has
//! no ties to a file system, a git repository or an operating system.
//!
//! That is the whole reason this crate exists: `joy-chat` describes what a
//! chat IS and must compile for the browser (JAPP-0135-FD), while these
//! types come from `joy-core`, which never will because it carries git.
//! Moving them down here is the smallest cut that frees the chat crate,
//! and `joy-core` re-exports them at their old paths, so nothing else
//! changes.

#![forbid(unsafe_code)]

// Nothing of the developer's shell and session reaches the unit tests
// of this crate (JOY-02BB-C7).
#[cfg(test)]
joy_test_env::isolate!();

pub mod interaction;
pub mod member_ref;

pub use interaction::InteractionLevel;
pub use member_ref::MemberRef;

/// Whether a member id names an AI member.
///
/// A person is known by an address, or in an anonymous project by an
/// opaque `m-` id. An AI member is known by its name, which is neither
/// (JI-019D-46): `claude`, `reviewer`. `m-` is reserved, so no name
/// starts with it. The form `ai:<name>@joy` that older projects carry
/// counts as well. THE one answer to this question for every crate.
pub fn is_ai_member(id: &str) -> bool {
    if id.starts_with("ai:") {
        return true;
    }
    !id.is_empty() && !id.contains('@') && !id.starts_with("m-")
}

/// The name of an AI member, whichever way its id is written: `claude`
/// for `claude` and for the older `ai:claude@joy` alike. A person's id
/// comes back as it is.
pub fn ai_member_name(id: &str) -> &str {
    id.strip_prefix("ai:")
        .map(|rest| rest.strip_suffix("@joy").unwrap_or(rest))
        .unwrap_or(id)
}

#[cfg(test)]
mod ai_member_ids {
    use super::{ai_member_name, is_ai_member};

    #[test]
    fn the_name_of_an_ai_member_is_the_same_in_both_spellings() {
        assert_eq!(ai_member_name("ai:claude@joy"), "claude");
        assert_eq!(ai_member_name("claude"), "claude");
        assert_eq!(ai_member_name("horst@joydev.com"), "horst@joydev.com");
    }

    #[test]
    fn a_name_is_an_ai_member_an_address_and_an_m_id_are_people() {
        for ai in ["claude", "reviewer", "ai:claude@joy", "ai:vibe@joy"] {
            assert!(is_ai_member(ai), "{ai}");
        }
        for person in ["horst@joydev.com", "m-nl6ts2ldoc", "m-abc", ""] {
            assert!(!is_ai_member(person), "{person:?}");
        }
    }
}
