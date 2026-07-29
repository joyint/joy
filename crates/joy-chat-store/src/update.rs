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
        let (done, skipped) = crate::migrations::apply(root)?;
        let mut rows = vec![RefreshRow {
            name: "chat storage".into(),
            action: if done.is_empty() {
                None
            } else {
                Some("sealed")
            },
        }];
        for s in skipped {
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
