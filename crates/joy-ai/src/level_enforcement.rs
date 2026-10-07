// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Interaction-level enforcement for `joy ai setup` (JI-0166-D8, JOY-0222-4E).
//!
//! Says which interaction level a tool is set up with and derives each
//! tool's NATIVE agent mode from it. The derivation is strictly one-way:
//! a native mode is never parsed back into a level and never persisted
//! in joy data; it exists only inside the generated tool configuration
//! files.
//!
//! The level is the AI member's project maximum (JI-019D-46): setup
//! artifacts are shared project truth, so what one person allows the
//! member for themselves does not go into them. A chat turn and a job
//! set their own level on the session when they start.

use joy_core::model::config::InteractionLevel;
use std::path::Path;

/// The level the tool of `member_id` is set up with in the project at
/// `root`.
///
/// With member files that is the level a manager signed for the member.
/// A member that is not registered yet (`joy ai init` writes the tool's
/// files first) gets the level a new AI member starts with; one whose
/// signature does not hold gets the most careful level. A project from
/// before the member files answers with the member's own level, else
/// the project default, as it did.
pub fn setup_level(root: &Path, member_id: &str) -> InteractionLevel {
    use joy_core::auth::{grants, vouch};
    let Ok(project) = joy_core::store::load_project(root) else {
        return joy_core::store::load_interaction_level_defaults(root).default;
    };
    let member = project.member_by_key(member_id);
    if !grants::applies(&project) {
        return member
            .and_then(|m| m.interaction_level)
            .unwrap_or_else(|| joy_core::store::load_interaction_level_defaults(root).default);
    }
    match member {
        None => vouch::DEFAULT_AI_LEVEL,
        Some(member) => match vouch::verify_maximum(&project, member_id, member) {
            Ok(()) => vouch::maximum_level(member),
            Err(_) => InteractionLevel::Proposing,
        },
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

/// The ACP session-mode ids that mean a given agent mode, across the
/// adapters we know (JOY-0280-A5): the lane picks the first one the
/// agent ADVERTISES in `available_modes` and sets it on the session at
/// every turn, so the level chosen for a turn reaches the tool itself and
/// not only the host's permission answers. An adapter that advertises
/// none of these keeps running under those answers alone.
///
/// The ids are the native strings the setup writers above emit, so the
/// table cannot drift from them (the tests hold both sides together).
pub fn session_mode_candidates(mode: joy_chat::model::AgentMode) -> &'static [&'static str] {
    use joy_chat::model::AgentMode;
    match mode {
        AgentMode::Plan => &["plan"],
        AgentMode::AcceptEdits => &["acceptEdits", "auto-edit"],
        AgentMode::Autonomous => &["bypassPermissions", "yolo"],
    }
}

/// The advertised mode id to set for `mode`, if the agent offers one.
pub fn pick_session_mode<'a>(
    advertised: impl IntoIterator<Item = &'a str>,
    mode: joy_chat::model::AgentMode,
) -> Option<&'static str> {
    let offered: Vec<&str> = advertised.into_iter().collect();
    session_mode_candidates(mode)
        .iter()
        .copied()
        .find(|id| offered.contains(id))
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

#[cfg(test)]
mod tests {
    use super::*;
    use joy_core::model::config::InteractionLevel::*;

    #[test]
    fn session_mode_candidates_agree_with_the_setup_writers() {
        use joy_chat::model::AgentMode;
        use InteractionLevel::*;
        for (level, mode) in [
            (Proposing, AgentMode::Plan),
            (Confirmed, AgentMode::AcceptEdits),
            (Autonomous, AgentMode::Autonomous),
        ] {
            let ids = session_mode_candidates(mode);
            assert!(ids.contains(&claude_permission_mode(level)), "{level:?}");
            assert!(ids.contains(&qwen_approval_mode(level)), "{level:?}");
        }
    }

    #[test]
    fn pick_session_mode_takes_what_the_agent_offers_and_nothing_else() {
        use joy_chat::model::AgentMode;
        let claude = ["default", "acceptEdits", "plan", "bypassPermissions"];
        assert_eq!(pick_session_mode(claude, AgentMode::Plan), Some("plan"));
        assert_eq!(
            pick_session_mode(claude, AgentMode::AcceptEdits),
            Some("acceptEdits")
        );
        assert_eq!(
            pick_session_mode(claude, AgentMode::Autonomous),
            Some("bypassPermissions")
        );
        let qwen = ["plan", "auto-edit", "yolo"];
        assert_eq!(pick_session_mode(qwen, AgentMode::Autonomous), Some("yolo"));
        // an adapter without modes: nothing to set, permission answers govern
        assert_eq!(pick_session_mode([], AgentMode::Autonomous), None);
        assert_eq!(pick_session_mode(["chat"], AgentMode::Plan), None);
    }

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

    /// The tool is set up with what a manager signed for the member; a
    /// member that is not there yet starts where a new one starts, and a
    /// level raised by hand sets the tool up as carefully as it gets.
    #[test]
    fn the_setup_level_is_the_signed_maximum() {
        use joy_core::auth::vouch::{self, Occasion};
        use joy_core::auth::IdentityKeypair;
        use joy_core::model::project::{Member, MemberCapabilities};

        let dir = tempfile::tempdir().unwrap();
        joy_core::init::init(joy_core::init::InitOptions {
            root: dir.path().to_path_buf(),
            name: Some("Setup".into()),
            acronym: Some("SU".into()),
            user: Some("dev@example.com".into()),
            language: None,
            host: joy_core::host::HostKind::Background,
            ask: None,
        })
        .unwrap();
        assert_eq!(setup_level(dir.path(), "claude"), vouch::DEFAULT_AI_LEVEL);

        let kp = IdentityKeypair::from_seed(&[9; 32]);
        let mut project = joy_core::store::load_project(dir.path()).unwrap();
        project
            .member_by_key_mut("dev@example.com")
            .unwrap()
            .verify_key = Some(kp.public_key().to_hex());
        let mut claude = Member::new(MemberCapabilities::Specific(
            [(joy_core::model::item::Capability::Plan, Default::default())].into(),
        ));
        claude.interaction_level = Some(Confirmed);
        vouch::sign(
            &project,
            "dev@example.com",
            &kp,
            "claude",
            &mut claude,
            Occasion::New,
        );
        project.register_member("claude", claude).unwrap();
        joy_core::store::save_project(dir.path(), &project).unwrap();
        assert_eq!(setup_level(dir.path(), "claude"), Confirmed);
        // the older spelling names the same member
        assert_eq!(setup_level(dir.path(), "ai:claude@joy"), Confirmed);

        let mut project = joy_core::store::load_project(dir.path()).unwrap();
        project
            .member_by_key_mut("claude")
            .unwrap()
            .interaction_level = Some(Autonomous);
        joy_core::store::save_project(dir.path(), &project).unwrap();
        assert_eq!(setup_level(dir.path(), "claude"), Proposing);
    }
}
