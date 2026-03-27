// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::path::Path;

use crate::identity;
use crate::model::item::Capability;
use crate::model::project::Project;
use crate::store;

/// Check whether the current user has a management capability.
/// Prints a warning to stderr if denied or not registered.
/// Returns true if allowed, false if denied.
pub fn warn_unless_capable(root: &Path, required: Capability) -> bool {
    let member_id = match identity::resolve_identity(root) {
        Ok(id) => id.member,
        Err(_) => return true, // Cannot determine user, allow
    };
    if member_id.is_empty() {
        return true;
    }

    let project_path = store::joy_dir(root).join(store::PROJECT_FILE);
    let project: Project = match store::read_yaml(&project_path) {
        Ok(p) => p,
        Err(_) => return true, // No project.yaml, allow
    };

    // No members configured means no restrictions
    if project.members.is_empty() {
        return true;
    }

    match project.members.get(&member_id) {
        Some(member) => {
            if member.has_capability(&required) {
                true
            } else {
                eprintln!(
                    "Warning: {} does not have '{}' capability. This action may be rejected by Joy Judge.",
                    member_id, required
                );
                false
            }
        }
        None => {
            eprintln!(
                "Warning: {} is not a registered project member. Run `joy project member add {}`.",
                member_id, member_id
            );
            false
        }
    }
}
