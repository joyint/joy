// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Rendering engine for AI tool integration files.
//!
//! Loads structured data (workflow, agents) and MiniJinja templates,
//! then renders complete, self-contained output files for each AI tool.
//! This replaces the previous approach of syncing intermediate files
//! to `.joy/ai/` and `.joy/capabilities/` (see ADR-024).

use minijinja::{context, Environment};

use crate::error::JoyError;

// ---------------------------------------------------------------------------
// Embedded data (YAML, parsed at runtime)
// ---------------------------------------------------------------------------

const WORKFLOW_DATA: &str = include_str!("../data/process/workflow.yaml");

const AGENT_CONCEIVER: &str = include_str!("../data/ai/agents/conceiver.yaml");
const AGENT_PLANNER: &str = include_str!("../data/ai/agents/planner.yaml");
const AGENT_DESIGNER: &str = include_str!("../data/ai/agents/designer.yaml");
const AGENT_IMPLEMENTER: &str = include_str!("../data/ai/agents/implementer.yaml");
const AGENT_TESTER: &str = include_str!("../data/ai/agents/tester.yaml");
const AGENT_REVIEWER: &str = include_str!("../data/ai/agents/reviewer.yaml");
const AGENT_DOCUMENTER: &str = include_str!("../data/ai/agents/documenter.yaml");

const ALL_AGENT_SOURCES: &[&str] = &[
    AGENT_CONCEIVER,
    AGENT_PLANNER,
    AGENT_DESIGNER,
    AGENT_IMPLEMENTER,
    AGENT_TESTER,
    AGENT_REVIEWER,
    AGENT_DOCUMENTER,
];

// ---------------------------------------------------------------------------
// Embedded templates (MiniJinja, rendered at runtime)
// ---------------------------------------------------------------------------

const INSTRUCTIONS_TMPL: &str = include_str!("../templates/ai/instructions.md");
const SETUP_TMPL: &str = include_str!("../templates/ai/instructions/setup.md");
const SKILL_TMPL: &str = include_str!("../templates/ai/skills/joy/SKILL.md");
const JOY_BLOCK_TMPL: &str = include_str!("../templates/ai/joy-block.md");

const CLAUDE_AGENT_TMPL: &str = include_str!("../templates/ai/tools/claude-code/agent.md");
const QWEN_AGENT_TMPL: &str = include_str!("../templates/ai/tools/qwen-code/agent.md");
const VIBE_AGENT_TMPL: &str = include_str!("../templates/ai/tools/mistral-vibe/agent.toml");
const COPILOT_AGENT_TMPL: &str =
    include_str!("../templates/ai/tools/github-copilot/agent.agent.md");
const COPILOT_PROMPT_TMPL: &str =
    include_str!("../templates/ai/tools/github-copilot/prompts/joy.prompt.md");

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load the workflow definition from embedded YAML.
pub fn load_workflow() -> Result<serde_json::Value, JoyError> {
    let value: serde_json::Value =
        serde_yaml_ng::from_str(WORKFLOW_DATA).map_err(|e| JoyError::Template(e.to_string()))?;
    Ok(value)
}

/// Load all agent definitions from embedded YAML.
pub fn load_agents() -> Result<Vec<serde_json::Value>, JoyError> {
    let mut agents = Vec::with_capacity(ALL_AGENT_SOURCES.len());
    for source in ALL_AGENT_SOURCES {
        let value: serde_json::Value =
            serde_yaml_ng::from_str(source).map_err(|e| JoyError::Template(e.to_string()))?;
        agents.push(value);
    }
    Ok(agents)
}

/// Render the joy-block (identity section inserted between markers in tool instruction files).
pub fn render_joy_block(member_id: &str, has_skill: bool) -> Result<String, JoyError> {
    let mut env = Environment::new();
    env.add_template("joy-block", JOY_BLOCK_TMPL)
        .map_err(|e| JoyError::Template(e.to_string()))?;
    let tmpl = env
        .get_template("joy-block")
        .map_err(|e| JoyError::Template(e.to_string()))?;
    let rendered = tmpl
        .render(context! {
            member_id => member_id,
            has_skill => has_skill,
        })
        .map_err(|e| JoyError::Template(e.to_string()))?;
    Ok(rendered.trim().to_string())
}

/// Render the full instructions.md with workflow context.
pub fn render_instructions(workflow: &serde_json::Value) -> Result<String, JoyError> {
    let mut env = Environment::new();
    env.add_template("instructions", INSTRUCTIONS_TMPL)
        .map_err(|e| JoyError::Template(e.to_string()))?;
    let tmpl = env
        .get_template("instructions")
        .map_err(|e| JoyError::Template(e.to_string()))?;
    let rendered = tmpl
        .render(context! { workflow => workflow })
        .map_err(|e| JoyError::Template(e.to_string()))?;
    Ok(rendered)
}

/// Render the SKILL.md with workflow context.
pub fn render_skill(workflow: &serde_json::Value) -> Result<String, JoyError> {
    let mut env = Environment::new();
    env.add_template("skill", SKILL_TMPL)
        .map_err(|e| JoyError::Template(e.to_string()))?;
    let tmpl = env
        .get_template("skill")
        .map_err(|e| JoyError::Template(e.to_string()))?;
    let rendered = tmpl
        .render(context! { workflow => workflow })
        .map_err(|e| JoyError::Template(e.to_string()))?;
    Ok(rendered)
}

/// Get the setup instructions content (no templating needed).
pub fn setup_instructions() -> &'static str {
    SETUP_TMPL
}

/// Agent template name for each supported tool.
fn agent_template_for_tool(tool: &str) -> Option<(&'static str, &'static str)> {
    match tool {
        "claude" => Some(("claude-agent", CLAUDE_AGENT_TMPL)),
        "qwen" => Some(("qwen-agent", QWEN_AGENT_TMPL)),
        "vibe" => Some(("vibe-agent", VIBE_AGENT_TMPL)),
        "copilot" => Some(("copilot-agent", COPILOT_AGENT_TMPL)),
        _ => None,
    }
}

/// Render an agent file for a specific tool.
pub fn render_agent(
    agent: &serde_json::Value,
    workflow: &serde_json::Value,
    tool: &str,
) -> Result<String, JoyError> {
    let (tmpl_name, tmpl_source) = agent_template_for_tool(tool)
        .ok_or_else(|| JoyError::Template(format!("Unknown tool: {tool}")))?;

    let mut env = Environment::new();
    env.add_template(tmpl_name, tmpl_source)
        .map_err(|e| JoyError::Template(e.to_string()))?;
    let tmpl = env
        .get_template(tmpl_name)
        .map_err(|e| JoyError::Template(e.to_string()))?;
    let rendered = tmpl
        .render(context! {
            agent => agent,
            workflow => workflow,
        })
        .map_err(|e| JoyError::Template(e.to_string()))?;
    Ok(rendered)
}

/// Render the Copilot skill wrapper (prompts/joy.prompt.md).
pub fn render_copilot_prompt(workflow: &serde_json::Value) -> Result<String, JoyError> {
    let mut env = Environment::new();
    env.add_template("copilot-prompt", COPILOT_PROMPT_TMPL)
        .map_err(|e| JoyError::Template(e.to_string()))?;
    let tmpl = env
        .get_template("copilot-prompt")
        .map_err(|e| JoyError::Template(e.to_string()))?;
    let rendered = tmpl
        .render(context! {
            workflow => workflow,
        })
        .map_err(|e| JoyError::Template(e.to_string()))?;
    Ok(rendered)
}

/// Check if an agent is applicable to a given tool.
pub fn agent_applicable_to_tool(agent: &serde_json::Value, tool: &str) -> bool {
    let tool_key = match tool {
        "claude" => "claude-code",
        "qwen" => "qwen-code",
        "vibe" => "mistral-vibe",
        "copilot" => "github-copilot",
        _ => return false,
    };
    agent["applicable_tools"]
        .as_array()
        .map(|tools| tools.iter().any(|t| t.as_str() == Some(tool_key)))
        .unwrap_or(false)
}

/// Get the agent name from an agent definition.
pub fn agent_name(agent: &serde_json::Value) -> Option<&str> {
    agent["name"].as_str()
}

/// Agent file extension for each tool.
pub fn agent_filename(agent: &serde_json::Value, tool: &str) -> Option<String> {
    let name = agent_name(agent)?;
    match tool {
        "claude" => Some(format!("{name}.md")),
        "qwen" => Some(format!("{name}.md")),
        "vibe" => Some(format!("{name}.toml")),
        "copilot" => Some(format!("{name}.agent.md")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_workflow_parses() {
        let wf = load_workflow().unwrap();
        let statuses = wf["statuses"].as_array().unwrap();
        assert_eq!(statuses.len(), 6);
        assert_eq!(statuses[0]["name"].as_str().unwrap(), "new");
    }

    #[test]
    fn load_agents_parses() {
        let agents = load_agents().unwrap();
        assert_eq!(agents.len(), 7);
        let names: Vec<&str> = agents.iter().filter_map(|a| a["name"].as_str()).collect();
        assert!(names.contains(&"implementer"));
        assert!(names.contains(&"reviewer"));
    }

    #[test]
    fn render_joy_block_contains_member_id() {
        let block = render_joy_block("ai:claude@joy", true).unwrap();
        assert!(block.contains("ai:claude@joy"));
        assert!(block.contains("/joy"));
    }

    #[test]
    fn render_joy_block_without_skill() {
        let block = render_joy_block("ai:copilot@joy", false).unwrap();
        assert!(block.contains("Joy CLI commands"));
        assert!(!block.contains("`/joy` skill"));
    }

    #[test]
    fn render_instructions_contains_workflow() {
        let wf = load_workflow().unwrap();
        let instructions = render_instructions(&wf).unwrap();
        assert!(instructions.contains("## Workflow"));
        assert!(instructions.contains("in-progress"));
        assert!(instructions.contains("review"));
        assert!(instructions.contains("joy start"));
    }

    #[test]
    fn render_skill_contains_workflow() {
        let wf = load_workflow().unwrap();
        let skill = render_skill(&wf).unwrap();
        assert!(skill.contains("### Workflow"));
        assert!(skill.contains("joy submit"));
    }

    #[test]
    fn render_claude_agent() {
        let wf = load_workflow().unwrap();
        let agents = load_agents().unwrap();
        let implementer = agents
            .iter()
            .find(|a| a["name"].as_str() == Some("implementer"))
            .unwrap();
        let rendered = render_agent(implementer, &wf, "claude").unwrap();
        assert!(rendered.contains("implementer"));
        assert!(rendered.contains("write, edit"));
    }

    #[test]
    fn render_vibe_agent() {
        let wf = load_workflow().unwrap();
        let agents = load_agents().unwrap();
        let reviewer = agents
            .iter()
            .find(|a| a["name"].as_str() == Some("reviewer"))
            .unwrap();
        let rendered = render_agent(reviewer, &wf, "vibe").unwrap();
        assert!(rendered.contains("display_name = \"reviewer\""));
        assert!(rendered.contains("safety = \"high\""));
    }

    #[test]
    fn agent_applicability() {
        let agents = load_agents().unwrap();
        let implementer = agents
            .iter()
            .find(|a| a["name"].as_str() == Some("implementer"))
            .unwrap();
        assert!(agent_applicable_to_tool(implementer, "claude"));
        assert!(agent_applicable_to_tool(implementer, "qwen"));

        let conceiver = agents
            .iter()
            .find(|a| a["name"].as_str() == Some("conceiver"))
            .unwrap();
        assert!(!agent_applicable_to_tool(conceiver, "qwen"));
    }

    #[test]
    fn render_copilot_prompt_contains_workflow() {
        let wf = load_workflow().unwrap();
        let prompt = render_copilot_prompt(&wf).unwrap();
        assert!(prompt.contains("## Workflow"));
    }

    // -----------------------------------------------------------------------
    // Integration tests: verify all generated files for all tools
    // -----------------------------------------------------------------------

    const ALL_TOOLS: &[&str] = &["claude", "qwen", "vibe", "copilot"];
    const WORK_AGENTS: &[&str] = &[
        "conceiver",
        "planner",
        "designer",
        "implementer",
        "tester",
        "reviewer",
        "documenter",
    ];

    #[test]
    fn workflow_has_all_statuses() {
        let wf = load_workflow().unwrap();
        let statuses = wf["statuses"].as_array().unwrap();
        let names: Vec<&str> = statuses.iter().filter_map(|s| s["name"].as_str()).collect();
        for expected in ["new", "open", "in-progress", "review", "closed", "deferred"] {
            assert!(names.contains(&expected), "missing status: {expected}");
        }
    }

    #[test]
    fn workflow_has_all_transitions() {
        let wf = load_workflow().unwrap();
        let transitions = wf["transitions"].as_array().unwrap();
        let expected = [
            ("new", "open"),
            ("open", "in-progress"),
            ("in-progress", "review"),
            ("review", "closed"),
            ("review", "in-progress"),
            ("deferred", "open"),
            ("closed", "open"),
        ];
        for (from, to) in expected {
            assert!(
                transitions
                    .iter()
                    .any(|t| { t["from"].as_str() == Some(from) && t["to"].as_str() == Some(to) }),
                "missing transition: {from} -> {to}"
            );
        }
    }

    #[test]
    fn workflow_transitions_have_capabilities() {
        let wf = load_workflow().unwrap();
        let transitions = wf["transitions"].as_array().unwrap();
        for t in transitions {
            assert!(
                t["capability"].as_str().is_some(),
                "transition {} -> {} missing capability",
                t["from"],
                t["to"]
            );
        }
    }

    #[test]
    fn all_agents_have_required_fields() {
        let agents = load_agents().unwrap();
        for agent in &agents {
            let name = agent["name"].as_str().expect("agent missing name");
            assert!(
                agent["capability"].as_str().is_some(),
                "{name} missing capability"
            );
            assert!(
                agent["description"].as_str().is_some(),
                "{name} missing description"
            );
            assert!(
                agent["default_mode"].as_str().is_some(),
                "{name} missing default_mode"
            );
            assert!(
                agent["permissions"]["allowed"].as_array().is_some(),
                "{name} missing permissions.allowed"
            );
            assert!(
                agent["permissions"]["denied"].as_array().is_some(),
                "{name} missing permissions.denied"
            );
            assert!(
                agent["constraints"].as_array().is_some(),
                "{name} missing constraints"
            );
            assert!(
                agent["applicable_tools"].as_array().is_some(),
                "{name} missing applicable_tools"
            );
        }
    }

    #[test]
    fn instructions_contain_all_sections() {
        let wf = load_workflow().unwrap();
        let instructions = render_instructions(&wf).unwrap();
        for section in [
            "## Session start",
            "## Identity and capabilities",
            "## Workflow",
            "## Core commands",
            "## Rules",
            "## Project context",
            "## Commit messages",
        ] {
            assert!(
                instructions.contains(section),
                "instructions missing section: {section}"
            );
        }
    }

    #[test]
    fn instructions_do_not_reference_joy_dir() {
        let wf = load_workflow().unwrap();
        let instructions = render_instructions(&wf).unwrap();
        assert!(
            !instructions.contains(".joy/ai/"),
            "instructions must not reference .joy/ai/"
        );
        assert!(
            !instructions.contains(".joy/capabilities/"),
            "instructions must not reference .joy/capabilities/"
        );
    }

    #[test]
    fn skill_contains_all_sections() {
        let wf = load_workflow().unwrap();
        let skill = render_skill(&wf).unwrap();
        for section in [
            "## Prerequisites",
            "## First session check",
            "### Viewing and navigating",
            "### Planning and creating items",
            "### Status changes",
            "### Workflow",
            "### Editing and organizing",
            "### Implementing items",
            "### Discovered bugs and ad-hoc fixes",
            "## General rules",
        ] {
            assert!(skill.contains(section), "skill missing section: {section}");
        }
    }

    #[test]
    fn skill_does_not_reference_joy_dir() {
        let wf = load_workflow().unwrap();
        let skill = render_skill(&wf).unwrap();
        assert!(
            !skill.contains(".joy/ai/instructions"),
            "skill must not reference .joy/ai/"
        );
    }

    #[test]
    fn skill_starts_with_yaml_frontmatter() {
        let wf = load_workflow().unwrap();
        let skill = render_skill(&wf).unwrap();
        assert!(
            skill.starts_with("---\n"),
            "skill must start with YAML frontmatter delimiter"
        );
        assert!(
            skill.contains("name: joy"),
            "skill must have name: joy in frontmatter"
        );
    }

    #[test]
    fn render_agent_for_all_tools() {
        let wf = load_workflow().unwrap();
        let agents = load_agents().unwrap();

        for tool in ALL_TOOLS {
            for agent in &agents {
                if !agent_applicable_to_tool(agent, tool) {
                    continue;
                }
                let name = agent_name(agent).unwrap();
                let rendered = render_agent(agent, &wf, tool)
                    .expect(&format!("failed to render {name} for {tool}"));
                assert!(!rendered.is_empty(), "empty render for {name}/{tool}");
                assert!(
                    rendered.contains(name),
                    "{name}/{tool}: rendered output missing agent name"
                );
            }
        }
    }

    #[test]
    fn md_agents_start_with_yaml_frontmatter() {
        let wf = load_workflow().unwrap();
        let agents = load_agents().unwrap();
        for tool in ["claude", "qwen"] {
            for agent in &agents {
                if !agent_applicable_to_tool(agent, tool) {
                    continue;
                }
                let name = agent_name(agent).unwrap();
                let rendered = render_agent(agent, &wf, tool).unwrap();
                assert!(
                    rendered.starts_with("---\n"),
                    "{name}/{tool}: must start with YAML frontmatter"
                );
            }
        }
    }

    #[test]
    fn vibe_agents_start_with_toml_section() {
        let wf = load_workflow().unwrap();
        let agents = load_agents().unwrap();
        for agent in &agents {
            if !agent_applicable_to_tool(agent, "vibe") {
                continue;
            }
            let name = agent_name(agent).unwrap();
            let rendered = render_agent(agent, &wf, "vibe").unwrap();
            assert!(
                rendered.starts_with("[agent]"),
                "{name}/vibe: must start with [agent] section, not a comment"
            );
        }
    }

    #[test]
    fn agent_filenames_have_correct_extensions() {
        let agents = load_agents().unwrap();
        for agent in &agents {
            let name = agent_name(agent).unwrap();
            for (tool, ext) in [
                ("claude", ".md"),
                ("qwen", ".md"),
                ("vibe", ".toml"),
                ("copilot", ".agent.md"),
            ] {
                if !agent_applicable_to_tool(agent, tool) {
                    continue;
                }
                let filename = agent_filename(agent, tool).unwrap();
                assert!(
                    filename.ends_with(ext),
                    "{name}/{tool}: expected extension {ext}, got {filename}"
                );
            }
        }
    }

    #[test]
    fn vibe_agents_have_toml_structure() {
        let wf = load_workflow().unwrap();
        let agents = load_agents().unwrap();
        for agent in &agents {
            if !agent_applicable_to_tool(agent, "vibe") {
                continue;
            }
            let name = agent_name(agent).unwrap();
            let rendered = render_agent(agent, &wf, "vibe").unwrap();
            assert!(
                rendered.contains("[agent]"),
                "{name}/vibe: missing [agent] section"
            );
            assert!(
                rendered.contains("display_name = "),
                "{name}/vibe: missing display_name"
            );
            assert!(
                rendered.contains("enabled_tools = "),
                "{name}/vibe: missing enabled_tools"
            );
        }
    }

    #[test]
    fn claude_agents_have_yaml_frontmatter() {
        let wf = load_workflow().unwrap();
        let agents = load_agents().unwrap();
        for agent in &agents {
            if !agent_applicable_to_tool(agent, "claude") {
                continue;
            }
            let name = agent_name(agent).unwrap();
            let rendered = render_agent(agent, &wf, "claude").unwrap();
            assert!(
                rendered.contains("---\nname:"),
                "{name}/claude: missing YAML frontmatter"
            );
        }
    }

    #[test]
    fn copilot_prompt_contains_all_sections() {
        let wf = load_workflow().unwrap();
        let prompt = render_copilot_prompt(&wf).unwrap();
        for section in ["## Status changes", "## Workflow", "## Implementing items"] {
            assert!(
                prompt.contains(section),
                "copilot prompt missing section: {section}"
            );
        }
    }

    #[test]
    fn reviewer_agent_has_restricted_permissions() {
        let agents = load_agents().unwrap();
        let reviewer = agents
            .iter()
            .find(|a| a["name"].as_str() == Some("reviewer"))
            .unwrap();
        let denied = reviewer["permissions"]["denied"].as_array().unwrap();
        let denied_strs: Vec<&str> = denied.iter().filter_map(|v| v.as_str()).collect();
        assert!(denied_strs.contains(&"write"), "reviewer must deny write");
        assert!(denied_strs.contains(&"edit"), "reviewer must deny edit");
    }

    #[test]
    fn implementer_agent_has_write_permissions() {
        let agents = load_agents().unwrap();
        let implementer = agents
            .iter()
            .find(|a| a["name"].as_str() == Some("implementer"))
            .unwrap();
        let allowed = implementer["permissions"]["allowed"].as_array().unwrap();
        let allowed_strs: Vec<&str> = allowed.iter().filter_map(|v| v.as_str()).collect();
        assert!(
            allowed_strs.contains(&"write"),
            "implementer must allow write"
        );
        assert!(
            allowed_strs.contains(&"edit"),
            "implementer must allow edit"
        );
        assert!(
            allowed_strs.contains(&"bash"),
            "implementer must allow bash"
        );
    }

    #[test]
    fn all_agent_names_covered() {
        let agents = load_agents().unwrap();
        let names: Vec<&str> = agents.iter().filter_map(|a| a["name"].as_str()).collect();
        for expected in WORK_AGENTS {
            assert!(names.contains(expected), "missing agent: {expected}");
        }
    }

    #[test]
    fn setup_instructions_not_empty() {
        let content = setup_instructions();
        assert!(!content.is_empty());
        assert!(content.contains("Vision"));
    }

    #[test]
    fn no_version_comments_in_rendered_output() {
        let wf = load_workflow().unwrap();
        let skill = render_skill(&wf).unwrap();
        assert!(
            !skill.contains("Generated by Joy"),
            "rendered output must not contain version comments"
        );

        let block = render_joy_block("ai:test@joy", true).unwrap();
        assert!(
            !block.contains("Generated by Joy"),
            "joy-block must not contain version comments"
        );

        let prompt = render_copilot_prompt(&wf).unwrap();
        assert!(
            !prompt.contains("Generated by Joy"),
            "copilot prompt must not contain version comments"
        );
    }

    // -----------------------------------------------------------------------
    // Line count enforcement (ADR-026: max 200 lines per generated file)
    // -----------------------------------------------------------------------

    const MAX_LINES: usize = 200;

    #[test]
    fn rendered_instructions_under_200_lines() {
        let wf = load_workflow().unwrap();
        let block = render_joy_block("ai:test@joy", true).unwrap();
        let instructions = render_instructions(&wf).unwrap();
        let combined = format!("{}\n\n{}", block, instructions);
        let lines = combined.lines().count();
        assert!(
            lines <= MAX_LINES,
            "instruction file would be {lines} lines (max {MAX_LINES})"
        );
    }

    #[test]
    fn rendered_skill_under_200_lines() {
        let wf = load_workflow().unwrap();
        let skill = render_skill(&wf).unwrap();
        let lines = skill.lines().count();
        assert!(
            lines <= MAX_LINES,
            "SKILL.md would be {lines} lines (max {MAX_LINES})"
        );
    }
}
