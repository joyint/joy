// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Canonical naming rules for AI members, riding on the ONE adapter
//! registry (JI-017A-85). Since JOY-0231-74 the adapter id IS the tool
//! name (`vibe`), exactly; recorded pins are kept current by the
//! official silent project.yaml migration. Every surface that derives a
//! member from an adapter goes through these helpers, never a string
//! split (the platform once derived a member `mistral` from
//! `mistral-vibe` that way).

/// The tool id an adapter string belongs to: the id itself for a
/// registered tool, `None` for `mock` and unknown adapters.
pub fn adapter_tool_id(adapter: &str) -> Option<&'static str> {
    crate::adapters::canonical_adapter_id(adapter)
}

/// The adapter id to RECORD for a tool: since JOY-0231-74 that is the
/// tool name itself, validated against the registry. `None` for an
/// unknown tool id.
pub fn tool_adapter(tool_id: &str) -> Option<&'static str> {
    crate::adapters::by_adapter(tool_id).map(|spec| spec.adapter)
}

/// The member a tool is registered as: the tool's name (`vibe`,
/// JI-019D-46). Adapters outside the registry (the test mock) use the
/// adapter name itself as the name.
pub fn member_id(adapter: &str) -> String {
    adapter_tool_id(adapter).unwrap_or(adapter).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_member_of_a_tool_is_the_tools_name() {
        assert_eq!(member_id("vibe"), "vibe");
        assert_eq!(member_id("claude"), "claude");
        assert_eq!(member_id("mock"), "mock");
    }

    #[test]
    fn the_recorded_adapter_is_the_tool_name_itself() {
        for tool in ["claude", "qwen", "vibe"] {
            assert_eq!(tool_adapter(tool), Some(tool));
            assert_eq!(adapter_tool_id(tool), Some(tool));
        }
        // first-generation spellings are the migration's business alone
        assert_eq!(adapter_tool_id("mistral-vibe"), None);
        assert_eq!(tool_adapter("mistral-vibe"), None);
        assert_eq!(adapter_tool_id("mock"), None);
        assert_eq!(tool_adapter("mock"), None);
    }
}
