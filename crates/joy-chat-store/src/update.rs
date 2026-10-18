// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! What the chat store contributes to joy's update path, and the list a
//! host runs when it opens a project.
//!
//! The framework and the repo artefacts live in `joy_core::update`; this
//! adds the one item that belongs to this crate and hands back the whole
//! set, so a host does not have to know which layer owns what. The CLI
//! adds its own tool files on top.

use joy_core::update::{
    CheckRow, Reach, RefreshRow, RowMark, UpdateItem, UpdateResult, SECTION_AUTH,
};
use std::path::Path;

/// Chat-store migrations (see `joy_chat_store::migrations`): an older
/// storage shape is brought into the sealed one.
///
/// No key is involved. Sealing wraps for the members' PUBLIC keys, so
/// this runs at the version sync like every other reconcile, and whoever
/// ran it still cannot read the chats afterwards. A chat that cannot be
/// converted is named, never half-converted.
struct ChatStoreMigrationItem;

impl UpdateItem for ChatStoreMigrationItem {
    fn reach(&self) -> Reach {
        Reach::Data
    }
    fn section(&self) -> &'static str {
        SECTION_AUTH
    }
    fn check(&self, root: &Path) -> UpdateResult<Vec<CheckRow>> {
        let waiting = crate::migrations::pending(root)?;
        Ok(vec![CheckRow {
            name: "chat storage".into(),
            mark: RowMark::from_ok(waiting.is_empty()),
            detail: if waiting.is_empty() {
                "up to date".into()
            } else {
                format!("{} chat(s) in the old shape", waiting.len())
            },
        }])
    }
    fn refresh(&self, root: &Path) -> UpdateResult<Vec<RefreshRow>> {
        let out = crate::migrations::apply(root)?;
        let mut rows = vec![RefreshRow {
            name: "chat storage".into(),
            action: if out.done.is_empty() {
                None
            } else {
                Some("sealed")
            },
        }];
        for d in out.dropped {
            rows.push(RefreshRow {
                name: format!("chat {}: {}", d.chat_id, d.why),
                action: Some("dropped"),
            });
        }
        for s in out.skipped {
            rows.push(RefreshRow {
                name: format!("chat {}: {}", s.chat_id, s.why),
                action: Some("left alone"),
            });
        }
        Ok(rows)
    }
}

/// Everything a project needs brought up to date when it is opened: the
/// repo artefacts joy-core owns plus the chat storage this crate owns.
pub fn project_items() -> Vec<Box<dyn UpdateItem>> {
    let mut items = joy_core::update::core_items();
    items.push(Box::new(ChatStoreMigrationItem));
    items
}

/// Bring a project up to date and say what changed.
///
/// `reach` decides how much: a person's own checkout gets everything a
/// `joy update` gives it, a server's clone only the data reconciles.
pub fn sync(root: &Path, reach: Reach) -> Vec<(&'static str, RefreshRow)> {
    let mut out = Vec::new();
    for item in project_items()
        .into_iter()
        .filter(|i| reach == Reach::Checkout || i.reach() == Reach::Data)
    {
        match item.refresh(root) {
            Ok(rows) => out.extend(rows.into_iter().map(|r| (item.section(), r))),
            Err(e) => out.push((
                item.section(),
                RefreshRow {
                    name: format!("{}: {e}", item.section()),
                    action: None,
                },
            )),
        }
    }
    out
}

/// What [`sync_and_commit`] did.
pub struct Synced {
    /// The rows of [`sync`].
    pub rows: Vec<(&'static str, RefreshRow)>,
    /// What the sync changed in the checkout.
    pub changed: joy_core::update::Changed,
    /// What became of those changes.
    pub commit: joy_core::git_ops::UpdateCommit,
}

/// [`sync`], and what it changed goes into one commit (JOY-02CF-EE).
///
/// For a host that opens a project for a person and has nobody to ask,
/// the desktop and the platform: `identity` names the person who opened
/// it, in the event-log form, and the commit is signed for them. What a
/// sync leaves uncommitted in a checkout is taken for a leftover, and on
/// a server it rides out in the commit of whatever a person does next.
pub fn sync_and_commit(root: &Path, reach: Reach, identity: &str) -> Synced {
    let before = joy_core::update::before(root);
    let rows = sync(root, reach);
    let changed = joy_core::update::changed_since(root, &before);
    let commit = joy_core::git_ops::commit_project_update(root, &changed, identity);
    Synced {
        rows,
        changed,
        commit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use joy_core::git_ops::UpdateCommit;
    use joy_core::vcs::forge;

    /// A project whose files are all committed and whose clone was last
    /// synced by an older joy.
    fn a_committed_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        joy_core::init::init(joy_core::init::InitOptions {
            name: Some("Synced".into()),
            acronym: Some("SY".into()),
            user: Some("horst@example.com".into()),
            ..joy_core::init::InitOptions::new(dir.path().to_path_buf())
        })
        .unwrap();
        sync(dir.path(), Reach::Checkout);
        forge::commit_everything(dir.path(), "init [no-item]", "m", "m@e.c").unwrap();
        joy_core::init::set_last_sync_version(dir.path(), "0.0.1").unwrap();
        dir
    }

    /// The host that opens a project gets the sync's changes as one
    /// commit, signed for the person who opened it, and their own files
    /// stay out of it (JOY-02CF-EE).
    #[test]
    fn opening_a_project_commits_what_the_sync_changed() {
        let dir = a_committed_project();
        let root = dir.path();
        // an older joy's SECURITY.md, and a file of the person's
        std::fs::write(root.join("SECURITY.md"), "what an older joy wrote\n").unwrap();
        forge::commit_everything(root, "older [no-item]", "m", "m@e.c").unwrap();
        std::fs::write(root.join("notes.txt"), "mine").unwrap();

        let synced = sync_and_commit(root, Reach::Checkout, "horst@example.com");
        let UpdateCommit::Written(id) = &synced.commit else {
            panic!("no commit: {:?}", synced.commit);
        };
        assert!(synced.changed.paths.contains(&"SECURITY.md".to_string()));
        assert_eq!(synced.changed.from.as_deref(), Some("0.0.1"));

        let repo = git2::Repository::open(root).unwrap();
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(&commit.id().to_string(), id);
        assert_eq!(commit.author().email().ok(), Some("horst@example.com"));
        let message = commit.message().unwrap_or_default().to_string();
        let subject = message.lines().next().unwrap_or_default();
        assert!(
            subject.starts_with("chore(joy): project files refreshed by joy "),
            "{subject}"
        );
        assert!(commit.tree().unwrap().get_name("notes.txt").is_none());
        assert_eq!(
            forge::checkout_picture(root).keys().collect::<Vec<_>>(),
            ["notes.txt"],
            "nothing of the sync is left in the checkout"
        );

        // opened again: nothing changed, nothing committed
        let again = sync_and_commit(root, Reach::Checkout, "horst@example.com");
        assert_eq!(again.commit, UpdateCommit::Nothing);
    }
}
