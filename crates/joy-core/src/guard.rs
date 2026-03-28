// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Centralized runtime validation for Joy's Trust Model.
//!
//! Guard intercepts write operations and checks them against the project's
//! member capabilities. It is the enforcement point for the Guardianship
//! pillar of AI Governance (ADR-021).
//!
//! Guard is zero-overhead for simple setups: when no members are configured,
//! or when the acting member has `capabilities: all`, checks return `Allow`
//! without capability mapping.

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::JoyError;
use crate::identity::Identity;
use crate::model::item::{Capability, Status};
use crate::model::project::{is_ai_member, Member, MemberCapabilities, Project};
use crate::store;

/// What operation is being attempted.
#[derive(Debug, Clone)]
pub enum Action {
    CreateItem,
    UpdateItem,
    DeleteItem,
    ChangeStatus {
        from: Status,
        to: Status,
    },
    AssignItem,
    AddComment,
    ManageProject,
    ManageMilestone,
    CreateRelease,
    StartJob {
        capability: Capability,
        estimated_cost: Option<f64>,
    },
}

impl Action {
    /// Map this action to the capability required to perform it.
    /// This is the authoritative source for the action-to-capability mapping.
    pub fn required_capability(&self) -> Capability {
        match self {
            Action::CreateItem => Capability::Create,
            Action::UpdateItem => Capability::Create,
            Action::DeleteItem => Capability::Delete,
            Action::AssignItem => Capability::Assign,
            Action::AddComment => Capability::Create,
            Action::ManageProject => Capability::Manage,
            Action::ManageMilestone => Capability::Manage,
            Action::CreateRelease => Capability::Manage,
            Action::ChangeStatus { to, .. } => match to {
                Status::InProgress => Capability::Implement,
                Status::Review => Capability::Review,
                Status::Closed => Capability::Review,
                Status::Deferred => Capability::Plan,
                Status::Open => Capability::Plan,
                Status::New => Capability::Create,
            },
            Action::StartJob { capability, .. } => *capability,
        }
    }
}

/// Result of a guard check.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Allow,
    Deny(String),
    Warn(String),
}

impl Verdict {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Verdict::Allow | Verdict::Warn(_))
    }

    pub fn is_denied(&self) -> bool {
        matches!(self, Verdict::Deny(_))
    }

    /// Convert this verdict into a Result, logging enforcement events.
    /// Deny becomes an error; Allow and Warn succeed.
    /// `target` is the item/milestone ID or "project" for management actions.
    pub fn enforce(self, root: &Path, target: &str, identity: &Identity) -> Result<(), JoyError> {
        match self {
            Verdict::Allow => Ok(()),
            Verdict::Warn(reason) => {
                crate::event_log::log_event_as(
                    root,
                    crate::event_log::EventType::GuardWarned,
                    target,
                    Some(&reason),
                    &identity.log_user(),
                );
                eprintln!("Warning: {reason}");
                Ok(())
            }
            Verdict::Deny(reason) => {
                crate::event_log::log_event_as(
                    root,
                    crate::event_log::EventType::GuardDenied,
                    target,
                    Some(&reason),
                    &identity.log_user(),
                );
                Err(JoyError::GuardDenied(reason))
            }
        }
    }
}

/// One-shot guard check: resolve identity, load project, check, enforce.
/// `author` is the optional `--author` CLI flag value.
pub fn enforce(
    root: &Path,
    action: &Action,
    target: &str,
    author: Option<&str>,
) -> Result<(), JoyError> {
    let identity = crate::identity::resolve_identity_with(root, author).unwrap_or(Identity {
        member: "unknown".into(),
        delegated_by: None,
        authenticated: false,
    });
    let project_path = store::joy_dir(root).join(store::PROJECT_FILE);
    let project: Project = store::read_yaml(&project_path)?;
    Guard::new(&project)
        .check(action, &identity)
        .enforce(root, target, &identity)
}

/// Centralized runtime validation for the Trust Model.
pub struct Guard {
    members: BTreeMap<String, Member>,
}

impl Guard {
    /// Create a Guard from a loaded project.
    pub fn new(project: &Project) -> Self {
        Self {
            members: project.members.clone(),
        }
    }

    /// Load project.yaml and create a Guard.
    pub fn load(root: &Path) -> Result<Self, JoyError> {
        let project_path = store::joy_dir(root).join(store::PROJECT_FILE);
        let project: Project = store::read_yaml(&project_path)?;
        Ok(Self::new(&project))
    }

    /// Check whether an action is allowed for the given identity.
    pub fn check(&self, action: &Action, identity: &Identity) -> Verdict {
        // No members configured: no restrictions
        if self.members.is_empty() {
            return Verdict::Allow;
        }

        // Look up the member
        let member = match self.members.get(&identity.member) {
            Some(m) => m,
            None => {
                return Verdict::Deny(format!(
                    "{} is not a registered project member",
                    identity.member
                ));
            }
        };

        // AI-specific gates apply regardless of capabilities (even capabilities: all)
        if is_ai_member(&identity.member) {
            let required = action.required_capability();

            // AI members are never allowed to perform manage actions
            if required == Capability::Manage {
                return Verdict::Deny(format!(
                    "AI member {} cannot perform manage actions",
                    identity.member
                ));
            }

            // Gate: AI members cannot close items (acceptance gate)
            if let Action::ChangeStatus {
                to: Status::Closed, ..
            } = action
            {
                return Verdict::Deny(format!(
                    "AI member {} cannot close items (acceptance gate)",
                    identity.member
                ));
            }
        }

        // Auth enforcement: manage actions require authentication when auth is active
        let auth_active = self.members.values().any(|m| m.public_key.is_some());
        if auth_active {
            let required = action.required_capability();
            if required == Capability::Manage && !identity.authenticated {
                return Verdict::Deny(format!(
                    "{} must authenticate to perform manage actions. Run `joy auth`.",
                    identity.member
                ));
            }
        }

        // Fast path: capabilities: all allows everything
        if member.capabilities == MemberCapabilities::All {
            return Verdict::Allow;
        }

        let required = action.required_capability();

        // Check if the member has the required capability
        if member.has_capability(&required) {
            // Budget pre-check for StartJob
            if let Action::StartJob {
                capability,
                estimated_cost: Some(cost),
            } = action
            {
                if let MemberCapabilities::Specific(ref map) = member.capabilities {
                    if let Some(config) = map.get(capability) {
                        if let Some(max_cost) = config.max_cost_per_job {
                            if *cost > max_cost {
                                return Verdict::Deny(format!(
                                    "{} estimated cost {:.2} exceeds max_cost_per_job {:.2} for '{}'",
                                    identity.member, cost, max_cost, capability
                                ));
                            }
                        }
                    }
                }
            }
            Verdict::Allow
        } else if required.is_management() {
            // Management actions are hard-denied (not just warned)
            Verdict::Deny(format!(
                "{} does not have '{}' capability",
                identity.member, required
            ))
        } else {
            Verdict::Warn(format!(
                "{} does not have '{}' capability. \
                 This action may be rejected by Joy Judge.",
                identity.member, required
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_log_dir(dir: &Path) {
        std::fs::create_dir_all(dir.join(".joy").join("logs")).unwrap();
    }

    fn identity(member: &str) -> Identity {
        Identity {
            member: member.into(),
            delegated_by: None,
            authenticated: false,
        }
    }

    fn ai_identity(member: &str, delegated_by: &str) -> Identity {
        Identity {
            member: member.into(),
            delegated_by: Some(delegated_by.into()),
            authenticated: false,
        }
    }

    fn project_with_members(members: Vec<(&str, MemberCapabilities)>) -> Project {
        let mut project = Project::new("Test".into(), Some("TST".into()));
        for (name, caps) in members {
            project.members.insert(name.into(), Member::new(caps));
        }
        project
    }

    fn specific_caps(caps: &[Capability]) -> MemberCapabilities {
        let map: BTreeMap<Capability, _> = caps
            .iter()
            .map(|c| (c.clone(), Default::default()))
            .collect();
        MemberCapabilities::Specific(map)
    }

    #[test]
    fn no_members_allows_all() {
        let project = Project::new("Test".into(), Some("TST".into()));
        let guard = Guard::new(&project);
        let id = identity("anyone@example.com");

        assert_eq!(guard.check(&Action::CreateItem, &id), Verdict::Allow);
        assert_eq!(guard.check(&Action::ManageProject, &id), Verdict::Allow);
        assert_eq!(guard.check(&Action::DeleteItem, &id), Verdict::Allow);
    }

    #[test]
    fn member_with_all_caps() {
        let project = project_with_members(vec![("dev@example.com", MemberCapabilities::All)]);
        let guard = Guard::new(&project);
        let id = identity("dev@example.com");

        assert_eq!(guard.check(&Action::CreateItem, &id), Verdict::Allow);
        assert_eq!(guard.check(&Action::ManageProject, &id), Verdict::Allow);
        assert_eq!(guard.check(&Action::DeleteItem, &id), Verdict::Allow);
        assert_eq!(
            guard.check(
                &Action::ChangeStatus {
                    from: Status::New,
                    to: Status::InProgress
                },
                &id
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn member_with_specific_caps() {
        let project = project_with_members(vec![(
            "dev@example.com",
            specific_caps(&[Capability::Implement, Capability::Create]),
        )]);
        let guard = Guard::new(&project);
        let id = identity("dev@example.com");

        // Has Create -> Allow
        assert_eq!(guard.check(&Action::CreateItem, &id), Verdict::Allow);

        // Has Implement -> ChangeStatus to InProgress = Allow
        assert_eq!(
            guard.check(
                &Action::ChangeStatus {
                    from: Status::Open,
                    to: Status::InProgress
                },
                &id
            ),
            Verdict::Allow
        );

        // Lacks Delete -> Deny (management capability)
        assert!(matches!(
            guard.check(&Action::DeleteItem, &id),
            Verdict::Deny(_)
        ));

        // Lacks Manage -> Deny (management actions are hard-denied)
        assert!(matches!(
            guard.check(&Action::ManageProject, &id),
            Verdict::Deny(_)
        ));

        // Lacks Review -> ChangeStatus to Closed = Warn
        assert!(matches!(
            guard.check(
                &Action::ChangeStatus {
                    from: Status::Review,
                    to: Status::Closed
                },
                &id
            ),
            Verdict::Warn(_)
        ));
    }

    #[test]
    fn ai_member_blocked_from_manage() {
        let project = project_with_members(vec![
            ("dev@example.com", MemberCapabilities::All),
            (
                "ai:claude@joy",
                specific_caps(&[
                    Capability::Implement,
                    Capability::Review,
                    Capability::Create,
                ]),
            ),
        ]);
        let guard = Guard::new(&project);
        let id = ai_identity("ai:claude@joy", "dev@example.com");

        // AI with Create -> CreateItem = Allow
        assert_eq!(guard.check(&Action::CreateItem, &id), Verdict::Allow);

        // AI with Implement -> ChangeStatus to InProgress = Allow
        assert_eq!(
            guard.check(
                &Action::ChangeStatus {
                    from: Status::Open,
                    to: Status::InProgress
                },
                &id
            ),
            Verdict::Allow
        );

        // AI attempting Manage -> Deny (regardless of capabilities)
        assert!(matches!(
            guard.check(&Action::ManageProject, &id),
            Verdict::Deny(_)
        ));
        assert!(matches!(
            guard.check(&Action::ManageMilestone, &id),
            Verdict::Deny(_)
        ));
        assert!(matches!(
            guard.check(&Action::CreateRelease, &id),
            Verdict::Deny(_)
        ));
    }

    #[test]
    fn ai_member_cannot_close_items() {
        let project = project_with_members(vec![
            ("dev@example.com", MemberCapabilities::All),
            (
                "ai:claude@joy",
                specific_caps(&[
                    Capability::Implement,
                    Capability::Review,
                    Capability::Create,
                ]),
            ),
        ]);
        let guard = Guard::new(&project);
        let ai = ai_identity("ai:claude@joy", "dev@example.com");
        let human = identity("dev@example.com");

        // AI cannot close items even with Review capability (acceptance gate)
        assert!(matches!(
            guard.check(
                &Action::ChangeStatus {
                    from: Status::Review,
                    to: Status::Closed
                },
                &ai
            ),
            Verdict::Deny(_)
        ));

        // AI can still submit for review
        assert_eq!(
            guard.check(
                &Action::ChangeStatus {
                    from: Status::InProgress,
                    to: Status::Review
                },
                &ai
            ),
            Verdict::Allow
        );

        // Human can close items
        assert_eq!(
            guard.check(
                &Action::ChangeStatus {
                    from: Status::Review,
                    to: Status::Closed
                },
                &human
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn unknown_member_denied() {
        let project = project_with_members(vec![("dev@example.com", MemberCapabilities::All)]);
        let guard = Guard::new(&project);
        let id = identity("stranger@example.com");

        assert!(matches!(
            guard.check(&Action::CreateItem, &id),
            Verdict::Deny(_)
        ));
    }

    #[test]
    fn status_transitions_require_correct_cap() {
        let project = project_with_members(vec![(
            "dev@example.com",
            specific_caps(&[Capability::Implement, Capability::Create]),
        )]);
        let guard = Guard::new(&project);
        let id = identity("dev@example.com");

        let check_transition = |to: Status| -> Verdict {
            guard.check(
                &Action::ChangeStatus {
                    from: Status::New,
                    to,
                },
                &id,
            )
        };

        // InProgress needs Implement -> Allow
        assert_eq!(check_transition(Status::InProgress), Verdict::Allow);
        // New needs Create -> Allow
        assert_eq!(check_transition(Status::New), Verdict::Allow);
        // Review needs Review -> Warn (missing)
        assert!(matches!(check_transition(Status::Review), Verdict::Warn(_)));
        // Closed needs Review -> Warn (missing)
        assert!(matches!(check_transition(Status::Closed), Verdict::Warn(_)));
        // Open needs Plan -> Warn (missing)
        assert!(matches!(check_transition(Status::Open), Verdict::Warn(_)));
        // Deferred needs Plan -> Warn (missing)
        assert!(matches!(
            check_transition(Status::Deferred),
            Verdict::Warn(_)
        ));
    }

    /// Integration test: realistic team with lead, developer, and AI agent.
    /// Verifies the full gate enforcement across a typical workflow.
    #[test]
    fn team_workflow_gate_enforcement() {
        let project = project_with_members(vec![
            // Lead: full access
            ("lead@example.com", MemberCapabilities::All),
            // Developer: can implement, test, create, but not review or manage
            (
                "dev@example.com",
                specific_caps(&[Capability::Implement, Capability::Test, Capability::Create]),
            ),
            // AI agent: can implement, review, create
            (
                "ai:claude@joy",
                specific_caps(&[
                    Capability::Implement,
                    Capability::Review,
                    Capability::Create,
                ]),
            ),
        ]);
        let guard = Guard::new(&project);

        let lead = identity("lead@example.com");
        let dev = identity("dev@example.com");
        let ai = ai_identity("ai:claude@joy", "lead@example.com");

        // === Creating items ===
        // All three can create (all have Create)
        assert_eq!(guard.check(&Action::CreateItem, &lead), Verdict::Allow);
        assert_eq!(guard.check(&Action::CreateItem, &dev), Verdict::Allow);
        assert_eq!(guard.check(&Action::CreateItem, &ai), Verdict::Allow);

        // === Starting work (-> InProgress needs Implement) ===
        let start = Action::ChangeStatus {
            from: Status::Open,
            to: Status::InProgress,
        };
        assert_eq!(guard.check(&start, &lead), Verdict::Allow);
        assert_eq!(guard.check(&start, &dev), Verdict::Allow);
        assert_eq!(guard.check(&start, &ai), Verdict::Allow);

        // === Submitting for review (-> Review needs Review) ===
        let submit = Action::ChangeStatus {
            from: Status::InProgress,
            to: Status::Review,
        };
        assert_eq!(guard.check(&submit, &lead), Verdict::Allow);
        // Dev lacks Review -> Warn
        assert!(matches!(guard.check(&submit, &dev), Verdict::Warn(_)));
        // AI has Review -> Allow
        assert_eq!(guard.check(&submit, &ai), Verdict::Allow);

        // === Closing items (-> Closed needs Review + acceptance gate) ===
        let close = Action::ChangeStatus {
            from: Status::Review,
            to: Status::Closed,
        };
        // Lead can close (capabilities: all)
        assert_eq!(guard.check(&close, &lead), Verdict::Allow);
        // Dev lacks Review -> Warn
        assert!(matches!(guard.check(&close, &dev), Verdict::Warn(_)));
        // AI has Review but CANNOT close (acceptance gate) -> Deny
        assert!(matches!(guard.check(&close, &ai), Verdict::Deny(_)));

        // === Managing project ===
        // Lead can manage
        assert_eq!(guard.check(&Action::ManageProject, &lead), Verdict::Allow);
        // Dev lacks Manage -> Deny (management actions are hard-denied)
        assert!(matches!(
            guard.check(&Action::ManageProject, &dev),
            Verdict::Deny(_)
        ));
        // AI cannot manage -> Deny
        assert!(matches!(
            guard.check(&Action::ManageProject, &ai),
            Verdict::Deny(_)
        ));
    }

    #[test]
    fn required_capability_mapping_is_complete() {
        // Verify every action maps to the expected capability
        assert_eq!(Action::CreateItem.required_capability(), Capability::Create);
        assert_eq!(Action::UpdateItem.required_capability(), Capability::Create);
        assert_eq!(Action::DeleteItem.required_capability(), Capability::Delete);
        assert_eq!(Action::AssignItem.required_capability(), Capability::Assign);
        assert_eq!(Action::AddComment.required_capability(), Capability::Create);
        assert_eq!(
            Action::ManageProject.required_capability(),
            Capability::Manage
        );
        assert_eq!(
            Action::ManageMilestone.required_capability(),
            Capability::Manage
        );
        assert_eq!(
            Action::CreateRelease.required_capability(),
            Capability::Manage
        );

        // Status transitions
        let cs = |to: Status| Action::ChangeStatus {
            from: Status::New,
            to,
        };
        assert_eq!(
            cs(Status::InProgress).required_capability(),
            Capability::Implement
        );
        assert_eq!(cs(Status::Review).required_capability(), Capability::Review);
        assert_eq!(cs(Status::Closed).required_capability(), Capability::Review);
        assert_eq!(cs(Status::Deferred).required_capability(), Capability::Plan);
        assert_eq!(cs(Status::Open).required_capability(), Capability::Plan);
        assert_eq!(cs(Status::New).required_capability(), Capability::Create);

        // StartJob delegates to its capability
        assert_eq!(
            Action::StartJob {
                capability: Capability::Implement,
                estimated_cost: None
            }
            .required_capability(),
            Capability::Implement
        );
    }

    #[test]
    fn verdict_enforce_allow() {
        let dir = tempfile::tempdir().unwrap();
        setup_log_dir(dir.path());
        let id = identity("dev@example.com");
        assert!(Verdict::Allow.enforce(dir.path(), "TST-0001", &id).is_ok());
    }

    #[test]
    fn verdict_enforce_deny_logs_event() {
        let dir = tempfile::tempdir().unwrap();
        setup_log_dir(dir.path());
        let id = identity("dev@example.com");
        let result = Verdict::Deny("blocked".into()).enforce(dir.path(), "TST-0001", &id);
        assert!(result.is_err());
        // Verify event was logged
        let events = crate::event_log::read_events(dir.path(), None, None, 100).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "guard.denied");
        assert_eq!(events[0].target, "TST-0001");
        assert_eq!(events[0].details.as_deref(), Some("blocked"));
    }

    #[test]
    fn verdict_enforce_warn_logs_event() {
        let dir = tempfile::tempdir().unwrap();
        setup_log_dir(dir.path());
        let id = identity("dev@example.com");
        let result = Verdict::Warn("caution".into()).enforce(dir.path(), "TST-0001", &id);
        assert!(result.is_ok());
        // Verify event was logged
        let events = crate::event_log::read_events(dir.path(), None, None, 100).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "guard.warned");
        assert_eq!(events[0].target, "TST-0001");
        assert_eq!(events[0].details.as_deref(), Some("caution"));
    }

    #[test]
    fn budget_precheck_allows_within_limit() {
        let mut caps = BTreeMap::new();
        caps.insert(
            Capability::Implement,
            crate::model::project::CapabilityConfig {
                max_mode: None,
                max_cost_per_job: Some(5.0),
            },
        );
        let project =
            project_with_members(vec![("ai:claude@joy", MemberCapabilities::Specific(caps))]);
        let guard = Guard::new(&project);
        let ai = ai_identity("ai:claude@joy", "dev@example.com");

        // Within budget -> Allow
        assert_eq!(
            guard.check(
                &Action::StartJob {
                    capability: Capability::Implement,
                    estimated_cost: Some(3.0),
                },
                &ai
            ),
            Verdict::Allow
        );

        // Exactly at limit -> Allow
        assert_eq!(
            guard.check(
                &Action::StartJob {
                    capability: Capability::Implement,
                    estimated_cost: Some(5.0),
                },
                &ai
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn budget_precheck_denies_over_limit() {
        let mut caps = BTreeMap::new();
        caps.insert(
            Capability::Implement,
            crate::model::project::CapabilityConfig {
                max_mode: None,
                max_cost_per_job: Some(5.0),
            },
        );
        let project =
            project_with_members(vec![("ai:claude@joy", MemberCapabilities::Specific(caps))]);
        let guard = Guard::new(&project);
        let ai = ai_identity("ai:claude@joy", "dev@example.com");

        // Over budget -> Deny
        let verdict = guard.check(
            &Action::StartJob {
                capability: Capability::Implement,
                estimated_cost: Some(7.50),
            },
            &ai,
        );
        assert!(matches!(verdict, Verdict::Deny(_)));
        if let Verdict::Deny(reason) = verdict {
            assert!(reason.contains("7.50"));
            assert!(reason.contains("5.00"));
        }
    }

    #[test]
    fn budget_precheck_allows_without_cost_limit() {
        let project = project_with_members(vec![(
            "ai:claude@joy",
            specific_caps(&[Capability::Implement]),
        )]);
        let guard = Guard::new(&project);
        let ai = ai_identity("ai:claude@joy", "dev@example.com");

        // No max_cost_per_job configured -> Allow regardless of cost
        assert_eq!(
            guard.check(
                &Action::StartJob {
                    capability: Capability::Implement,
                    estimated_cost: Some(999.0),
                },
                &ai
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn budget_precheck_allows_without_estimate() {
        let mut caps = BTreeMap::new();
        caps.insert(
            Capability::Implement,
            crate::model::project::CapabilityConfig {
                max_mode: None,
                max_cost_per_job: Some(5.0),
            },
        );
        let project =
            project_with_members(vec![("ai:claude@joy", MemberCapabilities::Specific(caps))]);
        let guard = Guard::new(&project);
        let ai = ai_identity("ai:claude@joy", "dev@example.com");

        // No estimated cost -> Allow (can't pre-check what we don't know)
        assert_eq!(
            guard.check(
                &Action::StartJob {
                    capability: Capability::Implement,
                    estimated_cost: None,
                },
                &ai
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn verdict_helpers() {
        assert!(Verdict::Allow.is_allowed());
        assert!(!Verdict::Allow.is_denied());

        assert!(Verdict::Warn("w".into()).is_allowed());
        assert!(!Verdict::Warn("w".into()).is_denied());

        assert!(!Verdict::Deny("d".into()).is_allowed());
        assert!(Verdict::Deny("d".into()).is_denied());
    }
}
