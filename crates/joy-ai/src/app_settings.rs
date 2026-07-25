// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Read access to the app-owned participant settings in
//! `.joy/app/app.yaml` (JI-0166-D8: the stored value is an interaction
//! LEVEL; the ACP agent mode is derived one-way and never persisted).
//!
//! The known shape (any unknown or malformed field is ignored):
//!
//! ```yaml
//! participants:
//!   ai:claude@joy:
//!     interaction-level: confirmed   # proposing | confirmed | autonomous
//!     entrypoint: "..."              # app-only, ignored here
//!     reach: { ... }                 # app-only, ignored here
//! ```
//!
//! Read-compat: a pre-2.0 file carries `mode:` with agent-mode names
//! (`plan | accept-edits | autonomous`); both the key and the values are
//! accepted on read and the app rewrites the new form on its next save.

use std::path::Path;

use joy_core::model::config::InteractionLevel;

/// The participant's default interaction level from `.joy/app/app.yaml`,
/// if the file exists and carries one for `member`. Every miss — no
/// file, a parse error, no such participant, an unknown value — is
/// `None`; the caller picks the fallback (the app treats a missing entry
/// as [`InteractionLevel::Proposing`]).
pub fn participant_default_level(root: &Path, member: &str) -> Option<InteractionLevel> {
    let path = root.join(".joy").join("app").join("app.yaml");
    let raw = std::fs::read_to_string(path).ok()?;
    // The app stores the file as JSON-compatible YAML; parsing as YAML
    // covers both spellings. Walk the document untyped so one malformed
    // sibling entry never hides the one asked for.
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&raw).ok()?;
    let participant = doc.get("participants")?.get(member)?;
    let value = participant
        .get("interaction-level")
        .or_else(|| participant.get("mode"))?
        .as_str()?;
    joy_chat::model::interaction::parse_level_compat(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use joy_core::model::config::InteractionLevel;

    fn write_app_yaml(root: &Path, content: &str) {
        let dir = root.join(".joy").join("app");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("app.yaml"), content).unwrap();
    }

    #[test]
    fn missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(participant_default_level(dir.path(), "ai:claude@joy"), None);
    }

    #[test]
    fn reads_the_level_and_ignores_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        write_app_yaml(
            dir.path(),
            "participants:\n  ai:claude@joy:\n    interaction-level: confirmed\n    entrypoint: claude --acp\n    reach:\n      agentEnabled: true\n",
        );
        assert_eq!(
            participant_default_level(dir.path(), "ai:claude@joy"),
            Some(InteractionLevel::Confirmed)
        );
        assert_eq!(
            participant_default_level(dir.path(), "ai:copilot@joy"),
            None
        );
    }

    #[test]
    fn reads_the_legacy_mode_key_and_values() {
        // A pre-2.0 file: key `mode`, agent-mode names.
        let dir = tempfile::tempdir().unwrap();
        write_app_yaml(
            dir.path(),
            "participants:\n  ai:claude@joy:\n    mode: accept-edits\n",
        );
        assert_eq!(
            participant_default_level(dir.path(), "ai:claude@joy"),
            Some(InteractionLevel::Confirmed)
        );
    }

    #[test]
    fn reads_the_json_spelling_the_app_writes() {
        // The app persists the file with serde_json (JSON is valid YAML).
        let dir = tempfile::tempdir().unwrap();
        write_app_yaml(
            dir.path(),
            r#"{
  "participants": {
    "ai:claude@joy": {
      "interaction-level": "autonomous",
      "entrypoint": "claude --acp"
    }
  }
}"#,
        );
        assert_eq!(
            participant_default_level(dir.path(), "ai:claude@joy"),
            Some(InteractionLevel::Autonomous)
        );
    }

    #[test]
    fn malformed_entries_are_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        // an unknown value, a structurally-off sibling, and a missing
        // level key: each miss is None, siblings stay readable
        write_app_yaml(
            dir.path(),
            "participants:\n  ai:claude@joy:\n    interaction-level: yolo\n  ai:copilot@joy: broken\n  ai:gemini@joy:\n    entrypoint: gemini\n  ai:codex@joy:\n    interaction-level: proposing\n",
        );
        assert_eq!(participant_default_level(dir.path(), "ai:claude@joy"), None);
        assert_eq!(
            participant_default_level(dir.path(), "ai:copilot@joy"),
            None
        );
        assert_eq!(participant_default_level(dir.path(), "ai:gemini@joy"), None);
        assert_eq!(
            participant_default_level(dir.path(), "ai:codex@joy"),
            Some(InteractionLevel::Proposing)
        );
    }
}
