// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Canonical naming rules for AI members: the mapping between an ACP
//! adapter (`mistral-vibe`), its tool id (`vibe`), and the canonical
//! member id (`ai:vibe@joy`). Every surface that derives a member from an
//! adapter must go through these helpers, never a string split (the
//! platform once derived `ai:mistral@joy` from `mistral-vibe` that way).

/// The tool id an ACP adapter belongs to (`mistral-vibe` -> `vibe`).
/// THE naming rule behind canonical AI members (`ai:<tool>@joy`): every
/// surface that derives a member from an adapter must use this mapping,
/// never a string split (the platform once derived `ai:mistral@joy` from
/// `mistral-vibe` that way). `mock` and unknown adapters have no tool.
pub fn adapter_tool_id(adapter: &str) -> Option<&'static str> {
    match adapter {
        "claude-code" => Some("claude"),
        "qwen-code" => Some("qwen"),
        "mistral-vibe" => Some("vibe"),
        "copilot" => Some("copilot"),
        _ => None,
    }
}

/// The ACP adapter a tool id runs on (`vibe` -> `mistral-vibe`): the inverse
/// of [`adapter_tool_id`]. The adapter is recorded on the project.yaml member
/// at registration (JI-0164), so the platform can route turns without a
/// per-member agent file. `None` for an unknown tool id.
pub fn tool_adapter(tool_id: &str) -> Option<&'static str> {
    match tool_id {
        "claude" => Some("claude-code"),
        "qwen" => Some("qwen-code"),
        "vibe" => Some("mistral-vibe"),
        "copilot" => Some("copilot"),
        _ => None,
    }
}

/// The canonical member id registered for an adapter (`ai:vibe@joy` for
/// `mistral-vibe`); adapters without a tool id (e.g. `mock`) use the
/// adapter name itself.
pub fn canonical_member_id(adapter: &str) -> String {
    format!("ai:{}@joy", adapter_tool_id(adapter).unwrap_or(adapter))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_canonical_member_follows_the_tool_not_the_provider() {
        assert_eq!(canonical_member_id("mistral-vibe"), "ai:vibe@joy");
        assert_eq!(canonical_member_id("claude-code"), "ai:claude@joy");
        assert_eq!(canonical_member_id("qwen-code"), "ai:qwen@joy");
        assert_eq!(canonical_member_id("mock"), "ai:mock@joy");
    }

    #[test]
    fn tool_adapter_is_the_inverse_of_adapter_tool_id() {
        for (adapter, tool) in [
            ("claude-code", "claude"),
            ("qwen-code", "qwen"),
            ("mistral-vibe", "vibe"),
            ("copilot", "copilot"),
        ] {
            assert_eq!(adapter_tool_id(adapter), Some(tool));
            assert_eq!(tool_adapter(tool), Some(adapter));
        }
        assert_eq!(adapter_tool_id("mock"), None);
        assert_eq!(tool_adapter("mock"), None);
    }
}
