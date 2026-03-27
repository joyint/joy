// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Identity resolution for Joy CLI operations.
//!
//! Resolves the acting user's identity by checking (in order):
//! 1. `--author` CLI flag (if provided)
//! 2. `JOY_AUTHOR` environment variable
//! 3. `git config user.email` (fallback)
//!
//! When the resolved identity is an AI member (`ai:*`), the git email
//! is captured as the delegating human (`delegated_by`).

use std::path::Path;

use crate::error::JoyError;
use crate::model::project::{is_ai_member, Project};
use crate::store;
use crate::vcs::Vcs;

/// The resolved identity of the acting user.
#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    /// The member ID (email or `ai:tool@joy`).
    pub member: String,
    /// If the member is an AI, the human who delegated the action.
    pub delegated_by: Option<String>,
}

impl Identity {
    /// Format for event log entries.
    /// Returns `"member"` or `"member delegated-by:human"`.
    pub fn log_user(&self) -> String {
        match &self.delegated_by {
            Some(human) => format!("{} delegated-by:{}", self.member, human),
            None => self.member.clone(),
        }
    }
}

/// Resolve the acting identity for the current operation.
///
/// Priority: `author_override` (--author flag) > `JOY_AUTHOR` env var > git email.
/// If the resolved identity is an AI member, the git email is used as `delegated_by`.
/// Validates that the identity is a registered project member (if members exist).
pub fn resolve_identity(root: &Path) -> Result<Identity, JoyError> {
    resolve_identity_with(root, None)
}

/// Like `resolve_identity`, but accepts an explicit `--author` override.
pub fn resolve_identity_with(
    root: &Path,
    author_override: Option<&str>,
) -> Result<Identity, JoyError> {
    let git_email = crate::vcs::default_vcs().user_email()?;
    let project = load_project_optional(root);

    // Priority: --author flag > JOY_AUTHOR env var > git email
    let override_author = author_override
        .map(|s| s.to_string())
        .or_else(|| std::env::var("JOY_AUTHOR").ok().filter(|s| !s.is_empty()));

    match override_author {
        Some(author) => {
            validate_member(&author, &project)?;
            let delegated_by = if is_ai_member(&author) {
                Some(git_email)
            } else {
                None
            };
            Ok(Identity {
                member: author,
                delegated_by,
            })
        }
        None => Ok(Identity {
            member: git_email,
            delegated_by: None,
        }),
    }
}

/// Check whether the project has any AI members.
pub fn has_ai_members(root: &Path) -> bool {
    let project = load_project_optional(root);
    match project {
        Some(p) => p.members.keys().any(|k| is_ai_member(k)),
        None => false,
    }
}

fn load_project_optional(root: &Path) -> Option<Project> {
    let project_path = store::joy_dir(root).join(store::PROJECT_FILE);
    store::read_yaml(&project_path).ok()
}

fn validate_member(member: &str, project: &Option<Project>) -> Result<(), JoyError> {
    let Some(project) = project else {
        return Ok(());
    };
    if project.members.is_empty() {
        return Ok(());
    }
    if !project.members.contains_key(member) {
        return Err(JoyError::Other(format!(
            "'{}' is not a registered project member. \
             Use `joy member add {}` to register.",
            member, member
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_log_user_simple() {
        let id = Identity {
            member: "alice@example.com".into(),
            delegated_by: None,
        };
        assert_eq!(id.log_user(), "alice@example.com");
    }

    #[test]
    fn identity_log_user_delegated() {
        let id = Identity {
            member: "ai:claude@joy".into(),
            delegated_by: Some("horst@joydev.com".into()),
        };
        assert_eq!(id.log_user(), "ai:claude@joy delegated-by:horst@joydev.com");
    }
}
