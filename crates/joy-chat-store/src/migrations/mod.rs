// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Chat-store migrations, the third kind next to joy-core's two.
//!
//! joy-core migrates project.yaml on read (`migrations::project_yaml`) and
//! reconciles the repo layout at sync time (`migrations::repo`). Chats
//! need the same corner, for the same reason: the storage shape moves with
//! the product, and what an older version wrote must not quietly fall out
//! of the app.
//!
//! Neither half needs a key, and that is the point. Sealing wraps the
//! epoch key for the MEMBERS' public keys; the private half is only ever
//! needed to READ. So whoever holds the repository can bring an old chat
//! into the current shape and still not be able to open it: the CLI does
//! it at the version sync, the platform does it when it loads a project,
//! and neither gains a way in.
//!
//! Each migration lives in a date-prefixed `m_<yyyy_mm>_<slug>.rs` module
//! and is removable in one step once its window closes: delete the file
//! and its lines in [`pending`] and [`apply`].

mod m_2026_07_sealed_chat_layout;

use std::path::Path;

use joy_core::error::JoyError;

/// One chat waiting for a migration, named so a command can say it out
/// loud instead of migrating behind the person's back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// The chat's id, as storage knows it.
    pub chat_id: String,
    /// What would happen to it, in one short line.
    pub what: &'static str,
}

/// One chat a migration actually converted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub chat_id: String,
    pub what: &'static str,
}

/// One chat a migration deliberately did NOT touch, and why. A chat that
/// cannot be converted must stay as it is and be named, never be
/// half-converted or silently emptied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub chat_id: String,
    pub why: String,
}

/// What is waiting, without touching a key. Safe to call anywhere,
/// including from a command that has no passphrase.
pub fn pending(root: &Path) -> Result<Vec<Pending>, JoyError> {
    m_2026_07_sealed_chat_layout::pending(root)
}

/// Run every chat migration, and say what was converted and what was
/// deliberately left alone.
///
/// No key: sealing wraps for the members' PUBLIC keys, so whoever holds
/// the repository can convert a chat without being able to read it
/// afterwards. That is what lets the platform do this by itself when a
/// new version arrives.
///
/// Idempotent: a chat already in the current shape is not in the list, so
/// calling this on every project load costs a tree read.
pub fn apply(root: &Path) -> Result<(Vec<Applied>, Vec<Skipped>), JoyError> {
    m_2026_07_sealed_chat_layout::apply(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use joy_chat::model::chat::{Chat, ChatMessage, MessageKind};
    use joy_core::auth::IdentityKeypair;
    use joy_core::member_ref::MemberRef;

    /// A project with one member who holds a key, and their seed.
    fn project() -> (tempfile::TempDir, [u8; 32]) {
        let dir = tempfile::tempdir().unwrap();
        joy_core::init::init(joy_core::init::InitOptions {
            root: dir.path().to_path_buf(),
            name: Some("Migrating".into()),
            acronym: Some("MG".into()),
            user: Some("horst@example.com".into()),
            language: None,
        })
        .unwrap();
        let seed = [7u8; 32];
        let mut project = joy_core::store::load_project(dir.path()).unwrap();
        project
            .member_by_key_mut("horst@example.com")
            .unwrap()
            .verify_key = Some(IdentityKeypair::from_seed(&seed).public_key().to_hex());
        joy_core::store::write_yaml(
            &joy_core::store::joy_dir(dir.path()).join(joy_core::store::PROJECT_FILE),
            &project,
        )
        .unwrap();
        (dir, seed)
    }

    /// A chat in the shape an older joy wrote: meta.yaml + messages/.
    fn legacy_chat(root: &std::path::Path, id: &str, text: &str) {
        let at = "2026-07-19T00:00:01Z".parse().unwrap();
        let mut chat = Chat::new(id, vec![MemberRef::new("horst@example.com")], at);
        chat.title = Some("Old room".into());
        chat.messages.push(ChatMessage {
            id: "m1".into(),
            at,
            author: MemberRef::new("horst@example.com"),
            text: text.into(),
            kind: MessageKind::Text,
            delegated_by: None,
            turn_ms: None,
            tool_steps: None,
            tool: None,
            payload: None,
            details: None,
        });
        crate::chat_ref::save_legacy_chat_for_tests(root, &chat);
    }

    #[test]
    fn a_repository_without_chats_has_nothing_to_migrate() {
        let (dir, _seed) = project();
        assert_eq!(pending(dir.path()).unwrap(), Vec::new());
        assert_eq!(apply(dir.path()).unwrap(), (Vec::new(), Vec::new()));
    }

    #[test]
    fn a_legacy_chat_is_named_without_a_key_and_sealed_with_one() {
        let (dir, seed) = project();
        let root = dir.path();
        legacy_chat(root, "aaaa0000aaaa0000aaaa0000aaaa0000", "was frueher war");

        // Named without any key: this is what a plain command can say.
        let waiting = pending(root).unwrap();
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].chat_id, "aaaa0000aaaa0000aaaa0000aaaa0000");
        // …and the apps see nothing for it, which is the whole problem
        assert!(crate::chat_store::snapshot(root, &waiting[0].chat_id)
            .unwrap()
            .is_none());

        let (done, skipped) = apply(root).unwrap();
        assert_eq!(done.len(), 1);
        assert!(skipped.is_empty(), "{skipped:?}");

        // now the transport serves it, the content survived, and nothing
        // is waiting any more
        assert!(crate::chat_store::snapshot(root, &waiting[0].chat_id)
            .unwrap()
            .is_some());
        let opened = crate::chat_store::load(root, &waiting[0].chat_id, &seed)
            .unwrap()
            .expect("the migrated chat opens with the member's own key");
        assert_eq!(opened.title.as_deref(), Some("Old room"));
        assert_eq!(opened.messages.len(), 1);
        assert_eq!(opened.messages[0].text, "was frueher war");
        assert_eq!(pending(root).unwrap(), Vec::new());
    }

    #[test]
    fn a_chat_this_build_cannot_read_is_left_alone_and_named() {
        // Between JOY-0218-E8 and the platform-key removal a message
        // carried its ciphertext in a field this model no longer knows.
        // Serde drops it, so the message reads as empty text. Sealing that
        // would replace a chat nobody can read with an EMPTY chat everyone
        // can read, which is worse than leaving it alone.
        let (dir, _seed) = project();
        let root = dir.path();
        let at = "2026-07-19T00:00:01Z".parse().unwrap();
        let mut chat = Chat::new(
            "cccc0000cccc0000cccc0000cccc0000",
            vec![MemberRef::new("horst@example.com")],
            at,
        );
        chat.messages.push(ChatMessage {
            id: "m1".into(),
            at,
            author: MemberRef::new("horst@example.com"),
            text: String::new(),
            kind: MessageKind::Text,
            delegated_by: None,
            turn_ms: None,
            tool_steps: None,
            tool: None,
            payload: None,
            details: None,
        });
        crate::chat_ref::save_legacy_chat_for_tests(root, &chat);

        let (done, skipped) = apply(root).unwrap();
        assert!(done.is_empty());
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].chat_id, chat.id);
        // …and it is still there, in the shape it was
        assert!(crate::chat_ref::load_chat(root, &chat.id)
            .unwrap()
            .is_some());
    }

    #[test]
    fn running_it_again_changes_nothing() {
        let (dir, _seed) = project();
        let root = dir.path();
        legacy_chat(root, "bbbb0000bbbb0000bbbb0000bbbb0000", "zweimal");
        assert_eq!(apply(root).unwrap().0.len(), 1);
        assert_eq!(apply(root).unwrap(), (Vec::new(), Vec::new()));
    }
}
