// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Canonical naming rules for AI members, riding on the ONE adapter
//! registry (JI-017A-85). Since JOY-0231-74 the adapter id IS the tool
//! name (`vibe`), exactly; recorded pins are kept current by the
//! official silent project.yaml migration. Every surface that derives a
//! member from an adapter goes through these helpers, never a string
//! split (the platform once derived `ai:mistral@joy` from
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

/// The id of the member a tool is registered as in `project`: the
/// tool's name with member files (`vibe`), `ai:vibe@joy` in a project
/// from before (JI-019D-46). Adapters outside the registry (the test
/// mock) use the adapter name itself as the name.
pub fn member_id(project: &joy_core::model::Project, adapter: &str) -> String {
    project.ai_member_id(adapter_tool_id(adapter).unwrap_or(adapter))
}

/// [`member_id`] for a caller that holds the root and not the project.
/// A project that cannot be read yet answers with the name.
pub fn member_id_at(root: &std::path::Path, adapter: &str) -> String {
    match joy_core::store::load_project(root) {
        Ok(project) => member_id(&project, adapter),
        Err(_) => adapter_tool_id(adapter).unwrap_or(adapter).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_member_of_a_tool_is_named_the_way_the_project_names_members() {
        let before = joy_core::model::Project::new("Old".into(), Some("OL".into()));
        assert_eq!(member_id(&before, "vibe"), "ai:vibe@joy");
        assert_eq!(member_id(&before, "mock"), "ai:mock@joy");

        // A project that keeps its members in files: read one back.
        let dir = tempfile::tempdir().unwrap();
        joy_core::init::init(joy_core::init::InitOptions {
            root: dir.path().to_path_buf(),
            name: Some("New".into()),
            acronym: Some("NW".into()),
            user: Some("founder@example.com".into()),
            language: None,
            host: joy_core::host::HostKind::Background,
            ask: None,
        })
        .unwrap();
        assert_eq!(member_id_at(dir.path(), "vibe"), "vibe");
        assert_eq!(member_id_at(dir.path(), "claude"), "claude");
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
