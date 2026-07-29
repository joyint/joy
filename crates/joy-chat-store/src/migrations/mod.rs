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
//! One thing makes these different, and it decides the whole design: a
//! chat migration WRITES A SEALED CHAT, so it needs the acting member's
//! key. The version sync runs on every joy invocation and usually has no
//! passphrase, so this module splits in two:
//!
//! - [`pending`] needs no key. It only reads the tree shape and says what
//!   is waiting, so any command can mention it.
//! - [`apply`] needs the seed and does the work. It runs where a key
//!   already is, which today means an authenticated `joy chat` command.
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

/// What is waiting, without touching a key. Safe to call anywhere,
/// including from a command that has no passphrase.
pub fn pending(root: &Path) -> Result<Vec<Pending>, JoyError> {
    m_2026_07_sealed_chat_layout::pending(root)
}

/// Run every chat migration that this seed can run.
///
/// Idempotent: a chat that is already in the current shape is skipped, so
/// calling this on every authenticated command costs a tree read.
pub fn apply(root: &Path, seed: &[u8; 32]) -> Result<Vec<Applied>, JoyError> {
    m_2026_07_sealed_chat_layout::apply(root, seed)
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
        crate::chat_ref::save_chat(root, &chat).unwrap();
    }

    #[test]
    fn a_repository_without_chats_has_nothing_to_migrate() {
        let (dir, seed) = project();
        assert_eq!(pending(dir.path()).unwrap(), Vec::new());
        assert_eq!(apply(dir.path(), &seed).unwrap(), Vec::new());
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

        let done = apply(root, &seed).unwrap();
        assert_eq!(done.len(), 1);

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
    fn running_it_again_changes_nothing() {
        let (dir, seed) = project();
        let root = dir.path();
        legacy_chat(root, "bbbb0000bbbb0000bbbb0000bbbb0000", "zweimal");
        assert_eq!(apply(root, &seed).unwrap().len(), 1);
        assert_eq!(apply(root, &seed).unwrap(), Vec::new());
    }
}
