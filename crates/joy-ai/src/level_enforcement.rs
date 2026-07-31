// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Interaction-level enforcement for `joy ai setup` (JI-0166-D8, JOY-0222-4E).
//!
//! Resolves an AI member's effective interaction levels from project data
//! and derives each tool's NATIVE agent mode from them. The derivation is
//! strictly one-way: a native mode is never parsed back into a level and
//! never persisted in joy data; it exists only inside the generated tool
//! configuration files.
//!
//! The resolution deliberately skips the personal-config layer: setup
//! artifacts are shared project truth, a developer's private preference in
//! `.joy/config.yaml` must not leak into them.

use joy_core::model::config::InteractionLevel;
use joy_core::model::item::Capability;
use joy_core::model::project::MemberCapabilities;
use std::path::Path;

/// The member's effective levels as far as setup enforcement needs them.
pub struct EnforcedLevels {
    /// The member's global effective level (member entry, else project
    /// defaults). Drives the tool-wide native mode.
    pub global: InteractionLevel,
    /// Effective level per held work capability, in `Capability::ALL` order.
    pub per_capability: Vec<(Capability, InteractionLevel)>,
}

/// Resolve the enforced levels for `member_id` in the project at `root`.
/// Falls back to the project defaults when the member does not exist yet
/// (initial `joy ai init` configures tools before registration completes).
pub fn resolve_for_member(root: &Path, member_id: &str) -> EnforcedLevels {
    let raw = joy_core::store::load_raw_interaction_level_defaults(root);
    let effective = joy_core::store::load_interaction_level_defaults(root);
    let project = joy_core::store::load_project(root).ok();
    let member = project.as_ref().and_then(|p| p.member_by_key(member_id));

    let member_global = member.and_then(|m| m.interaction_level);
    let global = member_global.unwrap_or(effective.default);

    let per_capability = Capability::ALL
        .iter()
        .filter(|c| c.is_work_capability())
        .filter(|c| member.is_none_or(|m| m.has_capability(c)))
        .map(|cap| {
            let cap_config = member.and_then(|m| match &m.capabilities {
                MemberCapabilities::Specific(map) => map.get(cap),
                MemberCapabilities::All => None,
            });
            let (level, _source) = joy_core::model::project::resolve_interaction_level(
                cap,
                &raw,
                &effective,
                member_global,
                None, // no personal layer in shared setup artifacts
                cap_config,
            );
            (*cap, level)
        })
        .collect();

    EnforcedLevels {
        global,
        per_capability,
    }
}

/// Claude Code native permission mode (`.claude/settings.json`
/// `permissions.defaultMode`). `confirmed` derives `acceptEdits`, not
/// `default`: acceptEdits auto-accepts reversible edits while bash keeps
/// prompting, which is exactly "confirm before irreversible actions".
pub fn claude_permission_mode(level: InteractionLevel) -> &'static str {
    match level {
        InteractionLevel::Proposing => "plan",
        InteractionLevel::Confirmed => "acceptEdits",
        InteractionLevel::Autonomous => "bypassPermissions",
    }
}

/// Qwen Code native approval mode (`.qwen/settings.json` `approvalMode`).
pub fn qwen_approval_mode(level: InteractionLevel) -> &'static str {
    match level {
        InteractionLevel::Proposing => "plan",
        InteractionLevel::Confirmed => "auto-edit",
        InteractionLevel::Autonomous => "yolo",
    }
}

/// Mistral Vibe native bash-tool permission (`.vibe/config.toml`
/// `[tools.bash] permission`). Vibe's repo config has no plan profile;
/// below `autonomous` every shell command is confirmed by the human.
pub fn vibe_bash_permission(level: InteractionLevel) -> &'static str {
    match level {
        InteractionLevel::Proposing | InteractionLevel::Confirmed => "ask",
        InteractionLevel::Autonomous => "always",
    }
}

/// One-line meaning of a level, shared by the managed-block section.
pub fn level_meaning(level: InteractionLevel) -> &'static str {
    match level {
        InteractionLevel::Autonomous => "work independently, governance gates are the checkpoints",
        InteractionLevel::Confirmed => "work independently, confirm before irreversible actions",
        InteractionLevel::Proposing => "propose, the human decides every step",
    }
}

/// The terse "Interaction levels" section of the managed instruction block.
/// Names the member's enforced global level, the native mode it derives for
/// the tool (when the tool has an enforceable surface), and per-capability
/// deviations from the global level.
pub fn managed_block_section(levels: &EnforcedLevels, tool: &str) -> String {
    let mut out = String::from("## Interaction levels\n\n");
    out.push_str(&format!(
        "Your interaction level: {} ({}).",
        levels.global,
        level_meaning(levels.global)
    ));
    let native = match tool {
        "claude" => Some(format!(
            " Enforced as Claude Code permission mode `{}`.",
            claude_permission_mode(levels.global)
        )),
        "qwen" => Some(format!(
            " Enforced as Qwen Code approval mode `{}`.",
            qwen_approval_mode(levels.global)
        )),
        "vibe" => Some(format!(
            " Enforced as Vibe bash permission `{}`.",
            vibe_bash_permission(levels.global)
        )),
        _ => None,
    };
    if let Some(native) = native {
        out.push_str(&native);
    }
    out.push('\n');

    let deviations: Vec<String> = levels
        .per_capability
        .iter()
        .filter(|(_, level)| *level != levels.global)
        .map(|(cap, level)| format!("{cap}: {level}"))
        .collect();
    if !deviations.is_empty() {
        out.push_str(&format!(
            "Per-capability deviations: {}.\n",
            deviations.join(", ")
        ));
    }
    out.push_str(
        "Levels: autonomous = gates only; confirmed = confirm irreversible actions; \
         proposing = human decides.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use joy_core::model::config::InteractionLevel::*;

    #[test]
    fn native_maps_are_one_way_and_total() {
        assert_eq!(claude_permission_mode(Proposing), "plan");
        assert_eq!(claude_permission_mode(Confirmed), "acceptEdits");
        assert_eq!(claude_permission_mode(Autonomous), "bypassPermissions");
        assert_eq!(qwen_approval_mode(Proposing), "plan");
        assert_eq!(qwen_approval_mode(Confirmed), "auto-edit");
        assert_eq!(qwen_approval_mode(Autonomous), "yolo");
        assert_eq!(vibe_bash_permission(Proposing), "ask");
        assert_eq!(vibe_bash_permission(Confirmed), "ask");
        assert_eq!(vibe_bash_permission(Autonomous), "always");
    }

    #[test]
    fn managed_block_section_names_global_and_deviations() {
        let levels = EnforcedLevels {
            global: Confirmed,
            per_capability: vec![
                (joy_core::model::item::Capability::Implement, Confirmed),
                (joy_core::model::item::Capability::Test, Autonomous),
            ],
        };
        let s = managed_block_section(&levels, "claude");
        assert!(s.contains("Your interaction level: confirmed"));
        assert!(s.contains("permission mode `acceptEdits`"));
        assert!(s.contains("test: autonomous"));
        assert!(!s.contains("implement: confirmed"), "no non-deviations");
    }

    #[test]
    fn resolve_prefers_member_entry_over_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let joy = joy_core::store::joy_dir(tmp.path());
        std::fs::create_dir_all(&joy).unwrap();
        std::fs::write(
            joy.join("project.defaults.yaml"),
            "interaction-level:\n  default: proposing\n  test: autonomous\n",
        )
        .unwrap();
        std::fs::write(
            joy.join("project.yaml"),
            "name: T\nlanguage: en\ncreated: \"2026-01-01T00:00:00+00:00\"\nmembers:\n  \
             ai:test@joy:\n    interaction-level: confirmed\n    capabilities:\n      \
             implement: {}\n      test: {}\n",
        )
        .unwrap();

        let levels = resolve_for_member(tmp.path(), "ai:test@joy");
        assert_eq!(levels.global, Confirmed);
        // Member global (confirmed) beats the project default (proposing)
        // for implement; the per-capability project default for test
        // (autonomous)... is itself overridden by the member global.
        let by_cap: std::collections::BTreeMap<_, _> =
            levels.per_capability.iter().cloned().collect();
        assert_eq!(
            by_cap[&joy_core::model::item::Capability::Implement],
            Confirmed
        );
        assert_eq!(by_cap[&joy_core::model::item::Capability::Test], Confirmed);
    }

    #[test]
    fn resolve_without_member_falls_back_to_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let joy = joy_core::store::joy_dir(tmp.path());
        std::fs::create_dir_all(&joy).unwrap();
        std::fs::write(
            joy.join("project.defaults.yaml"),
            "interaction-level:\n  default: confirmed\n",
        )
        .unwrap();
        let levels = resolve_for_member(tmp.path(), "ai:absent@joy");
        assert_eq!(levels.global, Confirmed);
        assert!(!levels.per_capability.is_empty());
    }

    #[test]
    fn copilot_has_no_native_enforcement_line() {
        let levels = EnforcedLevels {
            global: Proposing,
            per_capability: vec![],
        };
        let s = managed_block_section(&levels, "copilot");
        assert!(!s.contains("Enforced as"));
        assert!(s.contains("Your interaction level: proposing"));
    }
}
