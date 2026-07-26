// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! THE activity block (JI-0179-4F step 3): the persisted record of what a
//! turn DID — its thoughts, tool calls and answered permission
//! round-trips — as the versioned JSON the app's thread renders
//! (`parseDetails`, v1).
//!
//! Until 2026-07 this shape had three producers: the platform's chat
//! lanes, the platform's job rounds, and the desktop app's TypeScript
//! (which rebuilt it from streamed events and handed it back down). A
//! platform test tried to keep the copies equal by hand. One producer
//! now; TypeScript keeps only the parser and the live view.

/// What a turn did, as the host collected it from the agent's events.
#[derive(Debug, Default, Clone)]
pub struct Activity {
    /// Accumulated thinking text.
    pub thoughts: String,
    /// One entry per tool call: (title, last status).
    pub tools: Vec<(String, String)>,
    /// One entry per ANSWERED permission round-trip: (title, answer).
    pub permissions: Vec<(String, String)>,
}

impl Activity {
    /// The persisted v1 details JSON, or None when the turn had no
    /// activity worth a block (a plain text answer).
    pub fn to_details_json(&self) -> Option<String> {
        if self.thoughts.is_empty() && self.tools.is_empty() && self.permissions.is_empty() {
            return None;
        }
        let tools: Vec<serde_json::Value> = self
            .tools
            .iter()
            .map(|(title, status)| {
                serde_json::json!({ "title": title, "status": status.to_lowercase() })
            })
            .collect();
        let permissions: Vec<serde_json::Value> = self
            .permissions
            .iter()
            .map(|(title, answered)| serde_json::json!({ "title": title, "answered": answered }))
            .collect();
        Some(
            serde_json::json!({
                "v": 1,
                "thoughts": self.thoughts,
                "tools": tools,
                "permissions": permissions,
            })
            .to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_without_activity_has_no_block() {
        assert_eq!(Activity::default().to_details_json(), None);
    }

    #[test]
    fn the_block_matches_the_apps_parse_shape() {
        // The v1 contract the thread's parseDetails reads: v, thoughts,
        // tools[{title,status}], permissions[{title,answered}]; statuses
        // normalized to lowercase.
        let activity = Activity {
            thoughts: "brief reasoning".into(),
            tools: vec![("Read a".into(), "COMPLETED".into())],
            permissions: vec![("Edit b".into(), "allowed".into())],
        };
        let v: serde_json::Value =
            serde_json::from_str(&activity.to_details_json().expect("a block")).expect("json");
        assert_eq!(v["v"], 1);
        assert_eq!(v["thoughts"], "brief reasoning");
        assert_eq!(v["tools"][0]["title"], "Read a");
        assert_eq!(v["tools"][0]["status"], "completed");
        assert_eq!(v["permissions"][0]["title"], "Edit b");
        assert_eq!(v["permissions"][0]["answered"], "allowed");
    }
}
