// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Minimal read access to the desktop app's project policy file
//! `.joy/app/app.yaml` (JAPP-0031: `.joy/app` is the app's home for
//! shared, in-repo project settings). joy-core does NOT own this file —
//! the app writes it (as JSON-compatible YAML) and keeps its own full
//! reader/writer. This module extracts only what the PLATFORM needs to
//! resolve a turn's effective agent mode: the participant's project-wide
//! default mode (see [`joy_chat::model::agent_mode::effective_mode`]).
//!
//! The known shape (any unknown or malformed field is ignored):
//!
//! ```yaml
//! participants:
//!   ai:claude@joy:
//!     mode: accept-edits   # plan | accept-edits | autonomous
//!     entrypoint: "..."    # app-only, ignored here
//!     reach: { ... }       # app-only, ignored here
//! ```

use std::path::Path;

use joy_chat::model::agent_mode::AgentMode;

/// The participant's default agent mode from `.joy/app/app.yaml`, if the
/// file exists and carries one for `member`. Every miss — no file, a
/// parse error, no such participant, an unknown mode string — is `None`;
/// the caller picks the fallback (the app treats a missing entry as
/// [`AgentMode::Plan`]).
pub fn participant_default_mode(root: &Path, member: &str) -> Option<AgentMode> {
    let path = root.join(".joy").join("app").join("app.yaml");
    let raw = std::fs::read_to_string(path).ok()?;
    // The app stores the file as JSON-compatible YAML; parsing as YAML
    // covers both spellings. Walk the document untyped so one malformed
    // sibling entry never hides the one asked for.
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&raw).ok()?;
    doc.get("participants")?
        .get(member)?
        .get("mode")?
        .as_str()?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_app_yaml(root: &Path, content: &str) {
        let dir = root.join(".joy").join("app");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("app.yaml"), content).unwrap();
    }

    #[test]
    fn missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(participant_default_mode(dir.path(), "ai:claude@joy"), None);
    }

    #[test]
    fn reads_the_mode_and_ignores_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        write_app_yaml(
            dir.path(),
            "participants:\n  ai:claude@joy:\n    mode: accept-edits\n    entrypoint: claude --acp\n    reach:\n      agentEnabled: true\n",
        );
        assert_eq!(
            participant_default_mode(dir.path(), "ai:claude@joy"),
            Some(AgentMode::AcceptEdits)
        );
        assert_eq!(participant_default_mode(dir.path(), "ai:copilot@joy"), None);
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
      "mode": "autonomous",
      "entrypoint": "claude --acp"
    }
  }
}"#,
        );
        assert_eq!(
            participant_default_mode(dir.path(), "ai:claude@joy"),
            Some(AgentMode::Autonomous)
        );
    }

    #[test]
    fn malformed_entries_are_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        // an unknown mode string, a structurally-off sibling, and a
        // missing mode key: each miss is None, siblings stay readable
        write_app_yaml(
            dir.path(),
            "participants:\n  ai:claude@joy:\n    mode: yolo\n  ai:copilot@joy: broken\n  ai:gemini@joy:\n    entrypoint: gemini\n  ai:codex@joy:\n    mode: plan\n",
        );
        assert_eq!(participant_default_mode(dir.path(), "ai:claude@joy"), None);
        assert_eq!(participant_default_mode(dir.path(), "ai:copilot@joy"), None);
        assert_eq!(participant_default_mode(dir.path(), "ai:gemini@joy"), None);
        assert_eq!(
            participant_default_mode(dir.path(), "ai:codex@joy"),
            Some(AgentMode::Plan)
        );
    }
}
