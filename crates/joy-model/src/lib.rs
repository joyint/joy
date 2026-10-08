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
pub mod migrations;

pub use interaction::InteractionLevel;
pub use member_ref::MemberRef;

/// Whether a member id names an AI member.
///
/// A person is known by an address, or in an anonymous project by an
/// opaque `m-` id. An AI member is known by its name, which is neither
/// (JI-019D-46): `claude`, `reviewer`. `m-` is reserved, so no name
/// starts with it. An id in the legacy form that was not cut down where
/// it was read ([`migrations::ai_member_name`]) still counts as an AI
/// member's, so that such a one is never taken for a person. THE one
/// answer to this question for every crate.
pub fn is_ai_member(id: &str) -> bool {
    if migrations::ai_member_name::is_legacy(id) {
        return true;
    }
    !id.is_empty() && !id.contains('@') && !id.starts_with("m-")
}

#[cfg(test)]
mod ai_member_ids {
    use super::is_ai_member;

    #[test]
    fn a_name_is_an_ai_member_an_address_and_an_m_id_are_people() {
        for ai in ["claude", "reviewer"] {
            assert!(is_ai_member(ai), "{ai}");
        }
        for person in ["horst@joydev.com", "m-nl6ts2ldoc", "m-abc", ""] {
            assert!(!is_ai_member(person), "{person:?}");
        }
    }
}
