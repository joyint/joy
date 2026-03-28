// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Identity resolution for Joy CLI operations.
//!
//! Resolves the acting user's identity from:
//! 1. Active session (if one exists for any member)
//! 2. `git config user.email` (fallback for projects without auth)
//!
//! AI members authenticate via `joy auth --token`, which creates a
//! session. There is no self-declared identity override.

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
    /// Whether this identity was cryptographically authenticated (session or token).
    pub authenticated: bool,
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
/// Priority: active session > git email.
/// If a session exists, the identity comes from the session member.
/// If no session, falls back to git email (for projects without auth).
pub fn resolve_identity(root: &Path) -> Result<Identity, JoyError> {
    let git_email = crate::vcs::default_vcs().user_email()?;
    let project = load_project_optional(root);

    // Try to find an active session for the git user first,
    // then check if any AI session is active on this machine
    let project_id = crate::auth::session::project_id(root).ok();

    if let Some(ref pid) = project_id {
        // Check git user's session
        if let Some(session_identity) = session_identity(root, &git_email, pid, &project) {
            return Ok(session_identity);
        }

        // Check if there's an active AI session (for when AI tools call joy)
        if let Some(ref proj) = project {
            for member_id in proj.members.keys() {
                if is_ai_member(member_id) {
                    if let Some(session_identity) = session_identity(root, member_id, pid, &project)
                    {
                        return Ok(session_identity);
                    }
                }
            }
        }
    }

    // Fallback: git email, not authenticated
    Ok(Identity {
        member: git_email,
        delegated_by: None,
        authenticated: false,
    })
}

/// Try to build an Identity from an active session for a member.
fn session_identity(
    root: &Path,
    member: &str,
    project_id: &str,
    project: &Option<Project>,
) -> Option<Identity> {
    if !check_session(root, member, project) {
        return None;
    }

    // Read the session to get delegated_by info
    let delegated_by = crate::auth::session::load_session(project_id, member)
        .ok()
        .flatten()
        .and_then(|_sess| {
            // AI sessions have delegated_by from the token auth event
            if is_ai_member(member) {
                // The delegating human is tracked in the event log,
                // but for identity resolution we just mark it as delegated
                crate::vcs::default_vcs().user_email().ok()
            } else {
                None
            }
        });

    Some(Identity {
        member: member.to_string(),
        delegated_by,
        authenticated: true,
    })
}

/// Check whether the project has any AI members.
pub fn has_ai_members(root: &Path) -> bool {
    let project = load_project_optional(root);
    match project {
        Some(p) => p.members.keys().any(|k| is_ai_member(k)),
        None => false,
    }
}

/// Check if the member has an active, valid session.
fn check_session(root: &Path, member: &str, project: &Option<Project>) -> bool {
    let Some(project) = project else {
        return false;
    };
    if !project.members.contains_key(member) {
        return false;
    };
    let Ok(project_id) = crate::auth::session::project_id(root) else {
        return false;
    };
    let Ok(Some(sess)) = crate::auth::session::load_session(&project_id, member) else {
        return false;
    };

    // Check expiry and member match
    if sess.claims.expires <= chrono::Utc::now() || sess.claims.member != member {
        return false;
    }

    // For human members: validate session signature against public key
    if !is_ai_member(member) {
        let m = project.members.get(member).unwrap();
        let Some(ref pk_hex) = m.public_key else {
            return false;
        };
        let Ok(pk) = crate::auth::sign::PublicKey::from_hex(pk_hex) else {
            return false;
        };
        return crate::auth::session::validate_session(&sess, &pk, &project_id).is_ok();
    }

    // For AI members: session existence + not expired is sufficient
    // (token was validated at joy auth --token time)
    true
}

fn load_project_optional(root: &Path) -> Option<Project> {
    let project_path = store::joy_dir(root).join(store::PROJECT_FILE);
    store::read_yaml(&project_path).ok()
}

#[allow(dead_code)]
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
            authenticated: false,
        };
        assert_eq!(id.log_user(), "alice@example.com");
    }

    #[test]
    fn identity_log_user_delegated() {
        let id = Identity {
            member: "ai:claude@joy".into(),
            delegated_by: Some("horst@joydev.com".into()),
            authenticated: false,
        };
        assert_eq!(id.log_user(), "ai:claude@joy delegated-by:horst@joydev.com");
    }
}
