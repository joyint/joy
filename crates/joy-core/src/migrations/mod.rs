// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Schema migrations for joy-managed YAML files.
//!
//! Each migration is a pure transform on `serde_yaml_ng::Value`, isolated
//! in its own module under a date-prefixed filename. Migrations are
//! applied on read; the migrated form persists with the artifact's next
//! regular write — project.yaml at the next `joy auth update` (ADR-035),
//! an item when it is next saved — never as a mass rewrite.
//!
//! Removing a migration after its deprecation window is a one-step
//! operation: delete the module file and the corresponding entry from
//! the area-specific `apply` function. No cross-cutting changes needed.
//!
//! Migrations are grouped by kind so the mechanisms stay separate:
//! - [`project_yaml`]: pure on-read transforms of the parsed YAML value.
//! - [`item_yaml`]: the same discipline for item files.
//! - [`repo`]: filesystem-aware, one-shot reconciles run at sync time.
//! - [`ai_member_name`]: how the name of an AI member is read, in every
//!   file, chat, token and typed word alike.
//! - [`m_2026_10_member_files`]: the members from project.yaml into files
//!   of their own (JI-019D-46). The one migration that needs a person's
//!   key, so it is not applied on read or at sync time.
//! - [`m_2026_10_member_identity`]: the member files of joy 0.23 into
//!   format 4, `type` and `identity` (JOY-02DB-09). Asked like the one
//!   before, and run right after it in the same update.
//!
//! A migration that a person has to say yes to (it changes what an older
//! joy can read, and it is signed with their key) is reached only
//! through [`waiting`], [`update_with`], [`prepare_update`] and
//! [`finish_update`] below, with the words every
//! host shows for it: no command, server or app names its module. When
//! no project from before is left, removing it is deleting its module
//! and its lines in those functions.

use std::path::Path;

use crate::auth::IdentityKeypair;
use crate::error::JoyError;

/// Reading the name of an AI member, whichever form it was written or
/// typed in: THE one way, for every crate and every host.
pub use joy_model::migrations::ai_member_name;

pub mod item_yaml;
pub mod m_2026_10_member_files;
pub mod m_2026_10_member_identity;
pub mod project_yaml;
pub mod repo;

/// A project whose data is from an older joy and waits for a person to
/// update it: what the hosts ask and say, in their words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Waiting {
    /// The joy everybody needs once it is updated.
    pub needs_joy: &'static str,
}

impl Waiting {
    /// The question at a terminal, before `[Y/n]`.
    pub fn question(&self) -> String {
        format!(
            "Project data is from an older joy. Update? Everyone then needs joy {}+",
            self.needs_joy
        )
    }

    /// The two lines of the window an app shows when the project opens.
    pub fn window(&self) -> [String; 2] {
        [
            "Project data is from an older joy, some features don't work.".to_string(),
            format!(
                "Update? Everyone then needs joy {} or newer.",
                self.needs_joy
            ),
        ]
    }
}

/// What a feature says while the project waits: `what` is the feature,
/// as in "Your own model for claude".
pub fn needs_an_update(what: &str) -> String {
    format!("{what} needs an update of this project.")
}

/// Whether the project at `root` waits for a person to update its data.
/// Whatever waits, the update writes format 4, so everybody needs the
/// joy that reads it.
pub fn waiting(root: &Path) -> Option<Waiting> {
    if m_2026_10_member_files::pending(root) || m_2026_10_member_identity::pending(root) {
        return Some(Waiting {
            needs_joy: m_2026_10_member_identity::NEEDS_JOY,
        });
    }
    None
}

/// Update the project at `root`, signed by the person `signer_key` with
/// `keypair`, because they said yes. `Ok(false)`: nothing waited, or the
/// one who asked is not a person of this project with that key.
///
/// A project with its members still in project.yaml goes into member
/// files of format 4 in one step; one of joy 0.23 has its files
/// rewritten. Either way it is one update, and the host commits it once.
pub fn update_with(
    root: &Path,
    signer_key: &str,
    keypair: &IdentityKeypair,
) -> Result<bool, JoyError> {
    if m_2026_10_member_files::migrate(root, signer_key, keypair)? {
        return Ok(true);
    }
    m_2026_10_member_identity::migrate(root, signer_key)
}

/// An update converted and waiting for the person's signatures, for a
/// host whose person holds their key somewhere else (a browser).
pub struct PreparedUpdate(Prepared);

enum Prepared {
    MemberFiles(m_2026_10_member_files::Prepared),
    MemberIdentity(crate::model::project::Project),
}

impl PreparedUpdate {
    /// What the person signs, in the order the signatures are handed to
    /// [`finish_update`]. Rewriting member files signs nothing.
    pub fn payloads(&self) -> Vec<(String, Vec<u8>)> {
        match &self.0 {
            Prepared::MemberFiles(prepared) => prepared.payloads(),
            Prepared::MemberIdentity(_) => Vec::new(),
        }
    }
}

/// The first half of [`update_with`]: the converted project and what is
/// to be signed. `None`: nothing waits, or `signer_key` is not a person
/// of this project with a key.
pub fn prepare_update(root: &Path, signer_key: &str) -> Result<Option<PreparedUpdate>, JoyError> {
    if let Some(prepared) = m_2026_10_member_files::prepare(root, signer_key)? {
        return Ok(Some(PreparedUpdate(Prepared::MemberFiles(prepared))));
    }
    Ok(m_2026_10_member_identity::prepare(root, signer_key)?
        .map(|project| PreparedUpdate(Prepared::MemberIdentity(project))))
}

/// The second half: the signatures `signer_key` made over
/// [`PreparedUpdate::payloads`], in that order, each checked before
/// anything is written.
pub fn finish_update(
    root: &Path,
    prepared: PreparedUpdate,
    signer_key: &str,
    signatures: &[Vec<u8>],
) -> Result<(), JoyError> {
    match prepared.0 {
        Prepared::MemberFiles(prepared) => {
            m_2026_10_member_files::finish(root, prepared, signer_key, signatures)
        }
        Prepared::MemberIdentity(project) => {
            if !signatures.is_empty() {
                return Err(JoyError::Other(format!(
                    "nothing to sign, {} signatures",
                    signatures.len()
                )));
            }
            m_2026_10_member_identity::finish(root, &project)
        }
    }
}
