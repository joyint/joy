// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The non-interactive AI tool setup engine (ADR: shared logic lives in
//! joy-core). Everything here runs WITHOUT prompts or terminal output:
//! the CLI wraps it with questions and colored printing, the desktop app
//! calls it directly. `report` receives one line per touched path.

use joy_core::error::JoyError;
use std::fs;
use std::path::Path;

pub(crate) const JOY_BLOCK_START: &str = "<!-- joy:start -->";

pub(crate) const JOY_BLOCK_END: &str = "<!-- joy:end -->";

pub fn file_matches(path: &Path, expected: &str) -> bool {
    match fs::read_to_string(path) {
        Ok(content) => content == expected,
        Err(_) => false,
    }
}

pub fn remove_joy_block_or_file(path: &Path) -> Result<(), JoyError> {
    if !path.is_file() {
        return Ok(());
    }
    let existing = fs::read_to_string(path)?;
    let (Some(start), Some(end_pos)) =
        (existing.find(JOY_BLOCK_START), existing.find(JOY_BLOCK_END))
    else {
        return Ok(());
    };
    let end = end_pos + JOY_BLOCK_END.len();
    let mut remaining = String::new();
    remaining.push_str(&existing[..start]);
    remaining.push_str(&existing[end..]);
    if remaining.trim().is_empty() {
        fs::remove_file(path)?;
    } else {
        fs::write(path, format!("{}\n", remaining.trim_end()))?;
    }
    Ok(())
}

const TOOL_GITIGNORE_ENTRIES: &[(&str, &[(&str, &str)])] = &[
    ("claude", &[(".claude/", "Claude Code")]),
    ("qwen", &[(".qwen/", "Qwen Code")]),
    (
        "vibe",
        &[(".vibe/", "Mistral Vibe"), ("AGENTS.md", "Mistral Vibe")],
    ),
    (
        "copilot",
        &[
            (".github/copilot-instructions.md", "GitHub Copilot"),
            (".github/copilot/", "GitHub Copilot"),
            (".github/agents/", "GitHub Copilot"),
            (".github/prompts/", "GitHub Copilot"),
        ],
    ),
];

fn joy_block_matches(path: &Path, expected_block: &str) -> bool {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let expected_wrapped = format!("{}\n{}\n{}", JOY_BLOCK_START, expected_block, JOY_BLOCK_END);
    content.contains(&expected_wrapped)
}

fn update_qwen_permissions(root: &Path, member_id: &str, report: Report) -> Result<bool, JoyError> {
    let settings_path = root.join(".qwen/settings.json");

    let mut settings: serde_json::Value = if settings_path.is_file() {
        let content = fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Permissions
    let tools = settings
        .as_object_mut()
        .unwrap()
        .entry("tools")
        .or_insert_with(|| serde_json::json!({}));
    let allowed = tools
        .as_object_mut()
        .unwrap()
        .entry("allowed")
        .or_insert_with(|| serde_json::json!([]));
    let allowed_arr = allowed.as_array_mut().unwrap();
    for perm in ["run_shell_command(joy)", "run_shell_command(jyn)"] {
        if !allowed_arr.iter().any(|v| v.as_str() == Some(perm)) {
            allowed_arr.push(serde_json::json!(perm));
        }
    }

    // Enforced level -> native approval mode (JOY-0222-4E). Overwrites a
    // hand-edited value on purpose: project.yaml is the authority here.
    let levels = crate::level_enforcement::resolve_for_member(root, member_id);
    settings.as_object_mut().unwrap().insert(
        "approvalMode".into(),
        serde_json::json!(crate::level_enforcement::qwen_approval_mode(levels.global)),
    );

    let json = serde_json::to_string_pretty(&settings)?;
    let changed = write_if_changed(root, &settings_path, &format!("{json}\n"))?;
    report(".qwen/settings.json".into());

    Ok(changed)
}

fn ensure_vibe_bash_permission(
    root: &Path,
    path: &Path,
    member_id: &str,
) -> Result<bool, JoyError> {
    use toml_edit::{value, DocumentMut, Item, Table};

    let mut doc: DocumentMut = if path.is_file() {
        fs::read_to_string(path)?
            .parse()
            .map_err(|e: toml_edit::TomlError| JoyError::Other(e.to_string()))?
    } else {
        DocumentMut::new()
    };

    let tools = doc
        .entry("tools")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| JoyError::Other(".vibe/config.toml: [tools] is not a table".into()))?;
    tools.set_implicit(true);

    let bash = tools
        .entry("bash")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| JoyError::Other(".vibe/config.toml: [tools.bash] is not a table".into()))?;

    // Enforced level -> bash permission (JOY-0222-4E): `always` only for an
    // autonomous member, everyone else confirms each shell command. This
    // overwrites a hand-edited value on purpose (project.yaml is the
    // authority), which replaces the old keep-if-present behavior.
    let levels = crate::level_enforcement::resolve_for_member(root, member_id);
    let permission = crate::level_enforcement::vibe_bash_permission(levels.global);
    if bash.get("permission").and_then(|i| i.as_str()) == Some(permission) {
        return Ok(false);
    }
    bash["permission"] = value(permission);

    write_if_changed(root, path, &doc.to_string())
}

fn update_copilot_permissions(
    root: &Path,
    _member_id: &str,
    report: Report,
) -> Result<bool, JoyError> {
    let copilot_dir = root.join(".github/copilot");
    fs::create_dir_all(&copilot_dir)?;
    let settings_path = copilot_dir.join("settings.json");

    let mut settings: serde_json::Value = if settings_path.is_file() {
        let content = fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let allow = settings
        .as_object_mut()
        .unwrap()
        .entry("allowTools")
        .or_insert_with(|| serde_json::json!([]));

    let allow_arr = allow.as_array_mut().unwrap();

    for perm in ["shell(joy:*)", "shell(jyn:*)"] {
        if !allow_arr.iter().any(|v| v.as_str() == Some(perm)) {
            allow_arr.push(serde_json::json!(perm));
        }
    }

    let json = serde_json::to_string_pretty(&settings)?;
    let changed = write_if_changed(root, &settings_path, &format!("{json}\n"))?;
    report(".github/copilot/settings.json".into());

    Ok(changed)
}

fn git_path_is_tracked(root: &Path, path: &str) -> bool {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "--error-unmatch", "--", path])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    matches!(output, Ok(s) if s.code() == Some(0))
}

fn git_rm_cached(root: &Path, path: &str) -> bool {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rm", "--cached", "-r", "--quiet", "--", path])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match status {
        Ok(s) if s.success() => true,
        _ => {
            eprintln!("Warning: could not untrack {path}");
            false
        }
    }
}

fn git_rm_hard(root: &Path, path: &str) -> bool {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rm", "-r", "--quiet", "--ignore-unmatch", "--", path])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    matches!(status, Ok(s) if s.success())
}

#[cfg(test)]
fn existing_managed_block_entries(root: &Path) -> Vec<String> {
    use joy_core::init::{GITIGNORE_BLOCK_END, GITIGNORE_BLOCK_START};

    let path = root.join(".gitignore");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Some(start) = content.find(GITIGNORE_BLOCK_START) else {
        return Vec::new();
    };
    let after_start = start + GITIGNORE_BLOCK_START.len();
    let Some(end_offset) = content[after_start..].find(GITIGNORE_BLOCK_END) else {
        return Vec::new();
    };
    let block_body = &content[after_start..after_start + end_offset];
    block_body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// One line per touched file/action, for the caller to render.
pub type Report<'a> = &'a mut dyn FnMut(String);

pub fn is_tool_stale(root: &Path, tool: &str, member_id: &str) -> Result<bool, JoyError> {
    let workflow = crate::ai_templates::load_workflow()?;
    let agents = crate::ai_templates::load_agents()?;

    // Check SKILL.md (all tools except copilot)
    let skill_path = match tool {
        "claude" => Some(root.join(".claude/skills/joy/SKILL.md")),
        "qwen" => Some(root.join(".qwen/skills/joy/SKILL.md")),
        "vibe" => Some(root.join(".vibe/skills/joy/SKILL.md")),
        _ => None,
    };
    if let Some(path) = skill_path {
        let expected = crate::ai_templates::render_skill(&workflow)?;
        if !file_matches(&path, &expected) {
            return Ok(true);
        }
    }

    // Check setup.md (all tools except copilot)
    let setup_path = match tool {
        "claude" => Some(root.join(".claude/skills/joy/setup.md")),
        "qwen" => Some(root.join(".qwen/skills/joy/setup.md")),
        "vibe" => Some(root.join(".vibe/skills/joy/setup.md")),
        _ => None,
    };
    if let Some(path) = setup_path {
        if !file_matches(&path, crate::ai_templates::setup_instructions()) {
            return Ok(true);
        }
    }

    // Check instruction files (joy-block content)
    let block_path = match tool {
        "claude" => Some(root.join(".claude/CLAUDE.md")),
        "qwen" => Some(root.join(".qwen/QWEN.md")),
        "copilot" => Some(root.join(".github/copilot-instructions.md")),
        _ => None,
    };
    if let Some(path) = block_path {
        let has_skill = tool != "copilot";
        let expected_block = render_managed_block(root, member_id, has_skill, tool)?;
        if !joy_block_matches(&path, &expected_block) {
            return Ok(true);
        }
    }

    // Check copilot prompt
    if tool == "copilot" {
        let expected = crate::ai_templates::render_copilot_prompt(&workflow)?;
        if !file_matches(&root.join(".github/prompts/joy.prompt.md"), &expected) {
            return Ok(true);
        }
    }

    // Check agent files
    for agent in &agents {
        if !crate::ai_templates::agent_applicable_to_tool(agent, tool) {
            continue;
        }
        if let Some(filename) = crate::ai_templates::agent_filename(agent, tool) {
            let expected = crate::ai_templates::render_agent(agent, &workflow, tool)?;
            let agents_dir = match tool {
                "claude" => ".claude/agents",
                "qwen" => ".qwen/agents",
                "vibe" => ".vibe/agents",
                "copilot" => ".github/agents",
                _ => continue,
            };
            if !file_matches(&root.join(agents_dir).join(&filename), &expected) {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

pub struct MemberResetPlan {
    pub member_id: String,
    /// Drop the calling operator's own delegation to this AI.
    pub drop_caller_delegation: bool,
    /// Remove the shared member entry, because no delegation to this AI remains
    /// once the caller's is gone (the member is truly orphaned).
    pub remove_member: bool,
    /// A non-expired session for this member exists on this machine.
    pub active_session: bool,
    /// Delegations to this AI held by members other than the caller.
    pub other_delegators: usize,
}

pub fn plan_member_reset(
    project: &joy_core::model::Project,
    root: &Path,
    member_id: &str,
    caller_key: Option<&str>,
) -> Option<MemberResetPlan> {
    if !project.has_member_key(member_id) {
        return None;
    }
    let delegators: Vec<String> = project
        .members()
        .filter(|(_, m)| m.ai_delegations.contains_key(member_id))
        .map(|(k, _)| k.clone())
        .collect();
    let drop_caller_delegation =
        caller_key.is_some_and(|ck| delegators.iter().any(|d| d.as_str() == ck));
    let other_delegators = delegators
        .iter()
        .filter(|d| Some(d.as_str()) != caller_key)
        .count();
    let active_session = joy_core::auth::session::project_id(root)
        .ok()
        .is_some_and(|pid| joy_core::auth::session::has_active_session(&pid, member_id));
    Some(MemberResetPlan {
        member_id: member_id.to_string(),
        drop_caller_delegation,
        remove_member: other_delegators == 0,
        active_session,
        other_delegators,
    })
}

fn detect_claude() -> bool {
    which("claude")
}

fn detect_qwen() -> bool {
    which("qwen") || which("qwen-code")
}

fn detect_vibe() -> bool {
    which("vibe")
}

fn detect_copilot() -> bool {
    // Only the dedicated Copilot CLI counts. `gh` (the GitHub CLI) is present on
    // virtually every CI runner and many dev machines and says nothing about
    // whether Copilot is in use, so keying detection off it produced spurious
    // `ai:copilot@joy` registrations.
    which("copilot")
}

pub type ToolEntry = (
    &'static str,                                      // display name
    &'static str,                                      // id
    fn() -> bool,                                      // detect: installed?
    fn(&Path, &str, Report) -> Result<bool, JoyError>, // configure
);

pub const TOOLS: &[ToolEntry] = &[
    ("Claude Code", "claude", detect_claude, configure_claude),
    ("Qwen Code", "qwen", detect_qwen, configure_qwen),
    ("Mistral Vibe", "vibe", detect_vibe, configure_vibe),
    (
        "GitHub Copilot",
        "copilot",
        detect_copilot,
        configure_copilot,
    ),
];

/// Set up only NEW (not yet configured) tools. Returns list of all configured tool IDs.
fn render_managed_block(
    root: &Path,
    member_id: &str,
    has_skill: bool,
    tool: &str,
) -> Result<String, JoyError> {
    let workflow = crate::ai_templates::load_workflow()?;
    let joy_block = crate::ai_templates::render_joy_block(member_id, has_skill, tool)?;
    // The member's enforced levels live inside the managed block, so
    // `is_tool_stale` re-renders every tool artefact when a level changes
    // in project.yaml (JOY-0222-4E).
    let levels = crate::level_enforcement::resolve_for_member(root, member_id);
    let levels_section = crate::level_enforcement::managed_block_section(&levels, tool);
    let instructions = crate::ai_templates::render_instructions(&workflow)?;
    Ok(format!(
        "{}\n\n{}\n\n{}",
        joy_block,
        levels_section.trim_end(),
        instructions
    ))
}

/// Render SKILL.md with workflow context.
fn render_skill() -> Result<String, JoyError> {
    let workflow = crate::ai_templates::load_workflow()?;
    crate::ai_templates::render_skill(&workflow)
}

/// Remove and recreate Joy-managed subdirectories for a tool.
/// Preserves user-owned files (instruction files, settings.json).
pub fn clean_managed_dirs(root: &Path, dirs: &[&str]) {
    for dir in dirs {
        let path = root.join(dir);
        if path.is_dir() {
            let _ = fs::remove_dir_all(&path);
        }
    }
}

/// Generate agent files for a tool into the given directory.
pub fn generate_agents(
    root: &Path,
    tool: &str,
    agents_dir: &str,
    report: Report,
) -> Result<bool, JoyError> {
    let workflow = crate::ai_templates::load_workflow()?;
    let agents = crate::ai_templates::load_agents()?;
    let mut changed = false;

    for agent in &agents {
        if !crate::ai_templates::agent_applicable_to_tool(agent, tool) {
            continue;
        }
        if let Some(filename) = crate::ai_templates::agent_filename(agent, tool) {
            let content = crate::ai_templates::render_agent(agent, &workflow, tool)?;
            let path = root.join(agents_dir).join(&filename);
            changed |= write_if_changed(root, &path, &content)?;
            report(format!("{}/{}", agents_dir, filename));
        }
    }
    Ok(changed)
}

fn configure_claude(root: &Path, member_id: &str, report: Report) -> Result<bool, JoyError> {
    if !is_tool_stale(root, "claude", member_id)? {
        return Ok(false);
    }
    let claude_dir = root.join(".claude");
    fs::create_dir_all(&claude_dir)?;
    clean_managed_dirs(root, &[".claude/agents", ".claude/skills/joy"]);
    let mut changed = false;

    let claude_md = claude_dir.join("CLAUDE.md");
    changed |= update_with_joy_block(
        root,
        &claude_md,
        &render_managed_block(root, member_id, true, "claude")?,
    )?;
    report(".claude/CLAUDE.md".into());

    let skill_path = claude_dir.join("skills/joy/SKILL.md");
    changed |= write_if_changed(root, &skill_path, &render_skill()?)?;
    report(".claude/skills/joy/SKILL.md".into());

    let setup_path = claude_dir.join("skills/joy/setup.md");
    changed |= write_if_changed(root, &setup_path, crate::ai_templates::setup_instructions())?;
    report(".claude/skills/joy/setup.md".into());

    changed |= generate_agents(root, "claude", ".claude/agents", report)?;
    changed |= update_claude_permissions(root, member_id, report)?;

    Ok(changed)
}

fn update_claude_permissions(
    root: &Path,
    member_id: &str,
    report: Report,
) -> Result<bool, JoyError> {
    let settings_path = root.join(".claude/settings.json");

    let mut settings: serde_json::Value = if settings_path.is_file() {
        let content = fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Permissions
    let permissions = settings
        .as_object_mut()
        .unwrap()
        .entry("permissions")
        .or_insert_with(|| serde_json::json!({}));
    let allow = permissions
        .as_object_mut()
        .unwrap()
        .entry("allow")
        .or_insert_with(|| serde_json::json!([]));
    let allow_arr = allow.as_array_mut().unwrap();
    for perm in ["Bash(joy *)", "Bash(jyn *)"] {
        if !allow_arr.iter().any(|v| v.as_str() == Some(perm)) {
            allow_arr.push(serde_json::json!(perm));
        }
    }

    // Enforced level -> native permission mode (JOY-0222-4E). Overwrites a
    // hand-edited value on purpose: project.yaml is the authority here.
    let levels = crate::level_enforcement::resolve_for_member(root, member_id);
    permissions.as_object_mut().unwrap().insert(
        "defaultMode".into(),
        serde_json::json!(crate::level_enforcement::claude_permission_mode(
            levels.global
        )),
    );

    let json = serde_json::to_string_pretty(&settings)?;
    let changed = write_if_changed(root, &settings_path, &format!("{json}\n"))?;
    report(".claude/settings.json".into());

    Ok(changed)
}

fn configure_qwen(root: &Path, member_id: &str, report: Report) -> Result<bool, JoyError> {
    if !is_tool_stale(root, "qwen", member_id)? {
        return Ok(false);
    }
    let qwen_dir = root.join(".qwen");
    fs::create_dir_all(&qwen_dir)?;
    clean_managed_dirs(root, &[".qwen/agents", ".qwen/skills/joy"]);
    let mut changed = false;

    let qwen_md = qwen_dir.join("QWEN.md");
    changed |= update_with_joy_block(
        root,
        &qwen_md,
        &render_managed_block(root, member_id, true, "qwen")?,
    )?;
    report(".qwen/QWEN.md".into());

    let skill_path = qwen_dir.join("skills/joy/SKILL.md");
    changed |= write_if_changed(root, &skill_path, &render_skill()?)?;
    report(".qwen/skills/joy/SKILL.md".into());

    let setup_path = qwen_dir.join("skills/joy/setup.md");
    changed |= write_if_changed(root, &setup_path, crate::ai_templates::setup_instructions())?;
    report(".qwen/skills/joy/setup.md".into());

    changed |= generate_agents(root, "qwen", ".qwen/agents", report)?;
    changed |= update_qwen_permissions(root, member_id, report)?;

    Ok(changed)
}

fn configure_vibe(root: &Path, member_id: &str, report: Report) -> Result<bool, JoyError> {
    if !is_tool_stale(root, "vibe", member_id)? {
        return Ok(false);
    }
    let vibe_dir = root.join(".vibe");
    fs::create_dir_all(&vibe_dir)?;
    clean_managed_dirs(root, &[".vibe/agents", ".vibe/skills/joy"]);
    let mut changed = false;

    // Vibe reads {root}/AGENTS.md as "Project instructions" into its system
    // prompt (see mistral-vibe vibe/core/config/harness_files). `.vibe/AGENTS.md`
    // is NOT scanned, so the file must live at the workspace root.
    let agents_md = root.join("AGENTS.md");
    changed |= update_with_joy_block(
        root,
        &agents_md,
        &render_managed_block(root, member_id, true, "vibe")?,
    )?;
    report("AGENTS.md".into());

    let skill_path = vibe_dir.join("skills/joy/SKILL.md");
    changed |= write_if_changed(root, &skill_path, &render_skill()?)?;
    report(".vibe/skills/joy/SKILL.md".into());

    let setup_path = vibe_dir.join("skills/joy/setup.md");
    changed |= write_if_changed(root, &setup_path, crate::ai_templates::setup_instructions())?;
    report(".vibe/skills/joy/setup.md".into());

    changed |= generate_agents(root, "vibe", ".vibe/agents", report)?;

    let config_path = vibe_dir.join("config.toml");
    changed |= ensure_vibe_bash_permission(root, &config_path, member_id)?;
    report(".vibe/config.toml".into());

    Ok(changed)
}

fn configure_copilot(root: &Path, member_id: &str, report: Report) -> Result<bool, JoyError> {
    if !is_tool_stale(root, "copilot", member_id)? {
        return Ok(false);
    }
    let github_dir = root.join(".github");
    fs::create_dir_all(&github_dir)?;
    clean_managed_dirs(root, &[".github/agents", ".github/prompts"]);
    let mut changed = false;

    let instructions_md = github_dir.join("copilot-instructions.md");
    changed |= update_with_joy_block(
        root,
        &instructions_md,
        &render_managed_block(root, member_id, false, "copilot")?,
    )?;
    report(".github/copilot-instructions.md".into());

    // Copilot skill wrapper
    let workflow = crate::ai_templates::load_workflow()?;
    let prompt = crate::ai_templates::render_copilot_prompt(&workflow)?;
    let prompt_path = github_dir.join("prompts/joy.prompt.md");
    changed |= write_if_changed(root, &prompt_path, &prompt)?;
    report(".github/prompts/joy.prompt.md".into());

    changed |= generate_agents(root, "copilot", ".github/agents", report)?;
    changed |= update_copilot_permissions(root, member_id, report)?;

    Ok(changed)
}

pub fn write_if_changed(root: &Path, path: &Path, content: &str) -> Result<bool, JoyError> {
    if path.is_file() {
        let existing = fs::read_to_string(path)?;
        if existing == content {
            return Ok(false);
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    // No `auto_git_add` here. Every caller of `write_if_changed`
    // targets an AI-tool artefact (`.claude/`, `.vibe/`,
    // `.github/copilot-instructions.md`, `.github/prompts/`,
    // `.github/agents/`, `AGENTS.md`, etc.), all listed in the
    // joy-managed `.gitignore` block. Staging them would be a bug:
    // on the first `joy ai init` it would slip past `.gitignore`
    // (the block is rewritten *after* the tools are written), and
    // on every subsequent `joy update` it would fight the gitignore
    // and produce a wall of warnings. Joy-tracked artefacts
    // (project.yaml, docs templates, .gitignore, .gitattributes,
    // SECURITY.md, CONTRIBUTING.md) have their own explicit
    // `auto_git_add` calls elsewhere.
    let _ = root;
    Ok(true)
}

pub fn update_with_joy_block(root: &Path, path: &Path, content: &str) -> Result<bool, JoyError> {
    let block = format!("{}\n{}\n{}", JOY_BLOCK_START, content, JOY_BLOCK_END);

    let new_content = if path.is_file() {
        let existing = fs::read_to_string(path)?;
        if existing.contains(JOY_BLOCK_START) && existing.contains(JOY_BLOCK_END) {
            let start = existing.find(JOY_BLOCK_START).unwrap();
            let end = existing.find(JOY_BLOCK_END).unwrap() + JOY_BLOCK_END.len();
            let mut updated = String::new();
            updated.push_str(&existing[..start]);
            updated.push_str(&block);
            updated.push_str(&existing[end..]);
            updated
        } else {
            format!("{}\n\n{}", existing.trim_end(), block)
        }
    } else {
        format!("{}\n", block)
    };

    write_if_changed(root, path, &new_content)
}

/// The full, fixed set of entries in the joy-managed `.gitignore` block: the
/// base entries plus the ignore entries for every known AI tool. This is the
/// single source of truth for the block, shared by the `joy ai init` writer
/// ([`update_gitignore`]) and the `joy update` registry item that checks and
/// refreshes it, so the two paths can never drift (JOY-01FE-98).
pub fn managed_gitignore_entries() -> Vec<(&'static str, &'static str)> {
    let mut entries: Vec<(&'static str, &'static str)> =
        joy_core::init::GITIGNORE_BASE_ENTRIES.to_vec();
    for (_tool_id, tool_entries) in TOOL_GITIGNORE_ENTRIES {
        entries.extend_from_slice(tool_entries);
    }
    entries
}

pub fn update_gitignore(root: &Path, _configured_tools: &[&str]) -> Result<(), JoyError> {
    // Always write the full, fixed set: base entries plus the ignore entries
    // for every known AI tool, regardless of which tools are configured on
    // this machine. An ignore line for an absent directory is harmless, and
    // writing the complete set removes all per-machine / per-tool variance --
    // running `joy ai init` on a machine with fewer tools can no longer drop
    // entries another machine committed (JOY-01AA-9E). Because
    // `update_gitignore_block` is idempotent (it skips the write when the
    // content is unchanged), the per-invocation auto-sync produces no churn.
    joy_core::init::update_gitignore_block(root, &managed_gitignore_entries())?;
    Ok(())
}

pub fn untrack_gitignored_tool_files(root: &Path) {
    let mut untracked: Vec<&str> = Vec::new();
    for (_tool_id, tool_entries) in TOOL_GITIGNORE_ENTRIES {
        for (path, _comment) in *tool_entries {
            if git_path_is_tracked(root, path) && git_rm_cached(root, path) {
                untracked.push(path);
            }
        }
    }
    if !untracked.is_empty() {}
}

pub fn remove_legacy_ai_artifacts(root: &Path) -> Vec<String> {
    let mut removed = Vec::new();
    for rel in joy_core::init::LEGACY_AI_ARTIFACTS {
        let full = root.join(rel);
        if git_path_is_tracked(root, rel) && git_rm_hard(root, rel) && !full.exists() {
            removed.push((*rel).to_string());
            continue;
        }
        // Untracked-but-present (or git rm left it behind): remove directly.
        if full.exists() {
            let res = if full.is_dir() {
                fs::remove_dir_all(&full)
            } else {
                fs::remove_file(&full)
            };
            match res {
                Ok(()) => removed.push((*rel).to_string()),
                Err(e) => eprintln!("Warning: could not remove {rel}: {e}"),
            }
        }
    }

    // Prune an `.joy/ai/` that is now empty (nothing left to preserve).
    let ai_dir = root.join(".joy/ai");
    if ai_dir.is_dir()
        && fs::read_dir(&ai_dir)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false)
    {
        let _ = fs::remove_dir_all(&ai_dir);
    }

    removed
}

pub fn which(binary: &str) -> bool {
    std::process::Command::new("which")
        .arg(binary)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn is_tool_configured(root: &Path, tool: &str) -> bool {
    // Joy-only markers: paths that exist only because joy created them.
    // Generic instruction files (CLAUDE.md, QWEN.md, copilot-instructions.md)
    // can exist without any joy involvement and must not be detected as
    // "configured" -- otherwise joy ai init silently skips setup
    // (JOY-00D1-3C).
    match tool {
        "claude" => root.join(".claude/skills/joy/SKILL.md").is_file(),
        "qwen" => root.join(".qwen/skills/joy/SKILL.md").is_file(),
        "vibe" => root.join(".vibe/skills/joy/SKILL.md").is_file(),
        "copilot" => root.join(".github/agents/conceiver.agent.md").is_file(),
        _ => false,
    }
}

/// The acting human's identity, unlocked for attestations. No prompt:
/// the passphrase comes from the caller.
pub fn unlock_acting_keypair(
    project: &joy_core::model::Project,
    email: &str,
    passphrase: &str,
) -> Result<(String, joy_core::auth::IdentityKeypair), JoyError> {
    let member_key = joy_core::privacy::member_key_for_email(project, email)
        .ok_or_else(|| JoyError::Other(format!("{email} is not a registered project member")))?;
    let member = project
        .member_by_key(&member_key)
        .expect("member_key resolved from email must exist");
    if member.verify_key.is_none() {
        return Err(JoyError::Other(format!(
            "{email} has no registered public key. Run `joy auth init` first."
        )));
    }
    let unlocked = joy_core::auth::unlock_identity(member, passphrase)
        .map_err(|e| JoyError::Other(e.to_string()))?;
    Ok((member_key, unlocked.keypair))
}

/// Register the tool's AI member with an attestation when missing.
/// Returns whether project.yaml changed.
pub fn register_tool_member(
    project: &mut joy_core::model::Project,
    root: &Path,
    member_id: &str,
    attester: &(String, joy_core::auth::IdentityKeypair),
) -> Result<bool, JoyError> {
    if project.has_member_key(member_id) {
        return Ok(false);
    }
    let ai_defaults = joy_core::store::load_ai_defaults(root);
    let ai_caps = if ai_defaults.capabilities.is_empty() {
        joy_core::model::item::Capability::work_capabilities()
    } else {
        ai_defaults.capabilities.clone()
    };
    let capabilities = {
        use joy_core::model::project::CapabilityConfig;
        let mut map = std::collections::BTreeMap::new();
        for cap in ai_caps {
            map.insert(cap, CapabilityConfig::default());
        }
        joy_core::model::project::MemberCapabilities::Specific(map)
    };
    let (attester_id, attester_kp) = attester;
    let signed_fields =
        joy_core::auth::attestation::signed_fields_for(member_id, &capabilities, None);
    let attestation =
        joy_core::auth::attestation::sign_attestation(attester_id, attester_kp, signed_fields);
    let mut new_member = joy_core::model::project::Member::new(capabilities);
    new_member.attestation = Some(attestation);
    project
        .register_member(member_id, new_member)
        .map_err(|e| JoyError::Other(e.to_string()))?;
    Ok(true)
}

/// Configure ONE tool's files (idempotent; no registration).
pub fn configure_tool(root: &Path, tool: &str, report: Report) -> Result<bool, JoyError> {
    let spec = TOOLS
        .iter()
        .find(|(_, id, _, _)| *id == tool)
        .ok_or_else(|| JoyError::Other(format!("unknown tool: {tool}")))?;
    let member_id = format!("ai:{tool}@joy");
    (spec.3)(root, &member_id, report)
}

/// The desktop app's activation entry: configure the tool, register its
/// member (attested with the caller's passphrase), sync the gitignore.
pub fn init_tool(
    root: &Path,
    tool: &str,
    passphrase: &str,
    report: Report,
) -> Result<(), JoyError> {
    joy_core::embedded::sync_files(root, joy_core::init::PROJECT_FILES)?;
    configure_tool(root, tool, report)?;

    let project_path = joy_core::store::joy_dir(root).join(joy_core::store::PROJECT_FILE);
    let mut project = joy_core::store::read_project(&project_path)?;
    let member_id = format!("ai:{tool}@joy");
    if !project.has_member_key(&member_id) {
        let email = joy_core::event_log::get_git_email()?;
        let attester = unlock_acting_keypair(&project, &email, passphrase)?;
        if register_tool_member(&mut project, root, &member_id, &attester)? {
            joy_core::store::write_yaml_preserve(&project_path, &project)?;
            let rel = format!(
                "{}/{}",
                joy_core::store::JOY_DIR,
                joy_core::store::PROJECT_FILE
            );
            joy_core::git_ops::auto_git_add(root, &[&rel]);
            report(format!("{member_id} registered as member"));
        }
    }
    let configured: Vec<&'static str> = TOOLS
        .iter()
        .filter(|(_, id, _, _)| is_tool_configured(root, id))
        .map(|(_, id, _, _)| *id)
        .collect();
    update_gitignore(root, &configured)?;
    untrack_gitignored_tool_files(root);
    Ok(())
}

/// Per tool: joy-managed paths; shared instruction files keep the rest of
/// their content (only the joy block goes).
const RESET_PATHS: &[(&str, &str, &[&str])] = &[
    (
        "Claude Code",
        "claude",
        &[
            ".claude/skills/joy/",
            ".claude/agents/",
            ".claude/CLAUDE.md",
        ],
    ),
    (
        "Qwen Code",
        "qwen",
        &[".qwen/skills/joy/", ".qwen/agents/", ".qwen/QWEN.md"],
    ),
    (
        "Mistral Vibe",
        "vibe",
        &[".vibe/skills/joy/", ".vibe/agents/", "AGENTS.md"],
    ),
    (
        "GitHub Copilot",
        "copilot",
        &[
            ".github/copilot-instructions.md",
            ".github/agents/",
            ".github/prompts/",
        ],
    ),
];

/// Everything a reset WOULD do — the caller shows it and asks for consent.
pub struct ResetPlan {
    pub files: Vec<(&'static str, &'static str)>,
    pub member_plans: Vec<MemberResetPlan>,
    tools: Vec<(&'static str, &'static str, &'static [&'static str])>,
}

pub fn plan_reset(root: &Path, only: Option<&str>) -> Result<ResetPlan, JoyError> {
    let tools: Vec<_> = match only {
        Some(filter) => {
            let found = RESET_PATHS.iter().find(|(_, id, _)| *id == filter);
            match found {
                Some(t) => vec![*t],
                None => {
                    let valid: Vec<_> = RESET_PATHS.iter().map(|(_, id, _)| *id).collect();
                    return Err(JoyError::Other(format!(
                        "unknown tool: {filter}\nknown tools: {}",
                        valid.join(", ")
                    )));
                }
            }
        }
        None => RESET_PATHS.to_vec(),
    };
    let mut files = Vec::new();
    for (name, _, paths) in &tools {
        for path in *paths {
            if root.join(path).exists() {
                files.push((*name, *path));
            }
        }
    }
    let project = joy_core::store::read_project(
        &joy_core::store::joy_dir(root).join(joy_core::store::PROJECT_FILE),
    )
    .ok();
    let caller_key = project.as_ref().and_then(|_| {
        joy_core::identity::resolve_identity(root)
            .ok()
            .map(|id| id.member.id().to_string())
    });
    let mut member_plans = Vec::new();
    if let Some(ref p) = project {
        for (_, id, _) in &tools {
            let member_id = format!("ai:{id}@joy");
            if let Some(plan) = plan_member_reset(p, root, &member_id, caller_key.as_deref()) {
                if plan.drop_caller_delegation || plan.remove_member {
                    member_plans.push(plan);
                }
            }
        }
    }
    Ok(ResetPlan {
        files,
        member_plans,
        tools,
    })
}

/// Execute a consented reset plan. Returns the number of tools touched.
pub fn apply_reset(root: &Path, plan: &ResetPlan, report: Report) -> Result<usize, JoyError> {
    let project_path = joy_core::store::joy_dir(root).join(joy_core::store::PROJECT_FILE);
    let mut project = joy_core::store::read_project(&project_path).ok();
    let caller_key = project.as_ref().and_then(|_| {
        joy_core::identity::resolve_identity(root)
            .ok()
            .map(|id| id.member.id().to_string())
    });
    for (name, path) in &plan.files {
        let full = root.join(path);
        let is_shared_instruction = matches!(
            *path,
            ".claude/CLAUDE.md" | ".qwen/QWEN.md" | "AGENTS.md" | ".github/copilot-instructions.md"
        );
        if is_shared_instruction {
            remove_joy_block_or_file(&full)?;
        } else if full.is_dir() {
            fs::remove_dir_all(&full)?;
        } else {
            fs::remove_file(&full)?;
        }
        report(format!("{name} removed ({path})"));
    }
    if let Some(ref mut p) = project {
        let mut project_changed = false;
        for mp in &plan.member_plans {
            if mp.drop_caller_delegation {
                if let Some(ck) = caller_key.as_deref() {
                    if let Some(m) = p.member_by_key_mut(ck) {
                        if m.ai_delegations.remove(&mp.member_id).is_some() {
                            project_changed = true;
                        }
                    }
                }
            }
            if mp.remove_member {
                if p.remove_member(&mp.member_id).is_some() {
                    project_changed = true;
                    if let Ok(project_id) = joy_core::auth::session::project_id(root) {
                        let _ = joy_core::auth::session::remove_session(&project_id, &mp.member_id);
                    }
                    let member_keys: Vec<String> = p.member_keys().cloned().collect();
                    for k in &member_keys {
                        if let Some(m) = p.member_by_key_mut(k) {
                            m.ai_delegations.remove(&mp.member_id);
                        }
                    }
                    report(format!("{} member removed", mp.member_id));
                }
            } else if mp.drop_caller_delegation {
                report(format!("{} delegation removed (member kept)", mp.member_id));
            }
        }
        if project_changed {
            joy_core::store::write_yaml_preserve(&project_path, p)?;
            let rel = format!(
                "{}/{}",
                joy_core::store::JOY_DIR,
                joy_core::store::PROJECT_FILE
            );
            joy_core::git_ops::auto_git_add(root, &[&rel]);
        }
    }
    let any_remaining = RESET_PATHS
        .iter()
        .any(|(_, id, _)| is_tool_configured(root, id));
    if !any_remaining {
        joy_core::init::update_gitignore_block(root, joy_core::init::GITIGNORE_BASE_ENTRIES)?;
        let ai_dir = joy_core::store::joy_dir(root).join("ai");
        if ai_dir.exists() {
            let jobs_dir = ai_dir.join("jobs");
            let jobs_has_content = jobs_dir.is_dir()
                && fs::read_dir(&jobs_dir)
                    .map(|mut d| d.next().is_some())
                    .unwrap_or(false);
            if jobs_has_content {
                for entry in fs::read_dir(&ai_dir)? {
                    let entry = entry?;
                    if entry.file_name() != "jobs" {
                        let path = entry.path();
                        if path.is_dir() {
                            fs::remove_dir_all(&path)?;
                        } else {
                            fs::remove_file(&path)?;
                        }
                    }
                }
                report(".joy/ai/ cleaned (jobs/ preserved)".into());
            } else {
                fs::remove_dir_all(&ai_dir)?;
                report(".joy/ai/ removed".into());
            }
        }
    }
    let count = plan
        .tools
        .iter()
        .filter(|(_, _, paths)| {
            paths
                .iter()
                .any(|p| plan.files.iter().any(|(_, fp)| fp == p))
        })
        .count();
    Ok(count)
}

#[cfg(test)]
mod setup_tests {
    use super::*;

    #[test]
    fn existing_managed_block_entries_returns_empty_when_no_gitignore() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(existing_managed_block_entries(tmp.path()).is_empty());
    }

    #[test]
    fn managed_gitignore_entries_cover_base_and_every_tool() {
        // The single source of truth for the managed block must contain the
        // base entries and every AI tool's ignore entries, so the `joy update`
        // registry (which now derives its check/refresh from this set) can no
        // longer strip the per-tool lines (JOY-01FE-98).
        let paths: Vec<&str> = managed_gitignore_entries()
            .iter()
            .map(|(p, _)| *p)
            .collect();
        for (base, _) in joy_core::init::GITIGNORE_BASE_ENTRIES {
            assert!(
                paths.contains(base),
                "managed set missing base entry {base}"
            );
        }
        for (_tool, entries) in TOOL_GITIGNORE_ENTRIES {
            for (p, _) in *entries {
                assert!(paths.contains(p), "managed set missing tool entry {p}");
            }
        }
        for p in [".claude/", ".vibe/", "AGENTS.md"] {
            assert!(paths.contains(&p), "managed set missing {p}");
        }
    }

    fn seed_gitignore(root: &Path, managed_paths: &[&str]) {
        use joy_core::init::{GITIGNORE_BLOCK_END, GITIGNORE_BLOCK_START};
        let mut body = String::from(GITIGNORE_BLOCK_START);
        body.push('\n');
        for p in managed_paths {
            body.push_str(p);
            body.push('\n');
        }
        body.push_str(GITIGNORE_BLOCK_END);
        body.push('\n');
        fs::write(root.join(".gitignore"), body).unwrap();
    }

    #[test]
    fn remove_joy_block_preserves_user_content() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("AGENTS.md");
        fs::write(
            &path,
            format!(
                "user rule\n\n{}\nmanaged\n{}\n",
                JOY_BLOCK_START, JOY_BLOCK_END
            ),
        )
        .unwrap();
        remove_joy_block_or_file(&path).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("user rule"));
        assert!(!content.contains("managed"));
        assert!(!content.contains(JOY_BLOCK_START));
    }

    fn all_tool_paths() -> Vec<&'static str> {
        TOOL_GITIGNORE_ENTRIES
            .iter()
            .flat_map(|(_, entries)| entries.iter().map(|(p, _)| *p))
            .collect()
    }

    #[test]
    fn existing_managed_block_entries_returns_empty_when_no_block() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".gitignore"), "*.log\nnode_modules/\n").unwrap();
        assert!(existing_managed_block_entries(tmp.path()).is_empty());
    }

    #[test]
    fn remove_joy_block_deletes_file_when_only_block() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("AGENTS.md");
        fs::write(
            &path,
            format!("{}\nmanaged\n{}\n", JOY_BLOCK_START, JOY_BLOCK_END),
        )
        .unwrap();
        remove_joy_block_or_file(&path).unwrap();
        assert!(!path.exists());
    }

    fn deleg() -> joy_core::model::project::AiDelegationEntry {
        joy_core::model::project::AiDelegationEntry {
            delegation_verifier: "cc".repeat(32),
            delegation_salt: None,
            created: chrono::DateTime::parse_from_rfc3339("2026-04-15T10:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            rotated: None,
        }
    }

    fn read_block_lines(root: &Path) -> Vec<String> {
        existing_managed_block_entries(root)
    }

    fn project_with(ai: &str, delegators: &[&str]) -> joy_core::model::Project {
        use joy_core::model::{Member, MemberCapabilities, Project};
        let mut p = Project::new("Test".to_string(), Some("TS".to_string()));
        p.register_member(ai, Member::new(MemberCapabilities::All))
            .unwrap();
        for d in delegators {
            let mut m = Member::new(MemberCapabilities::All);
            m.ai_delegations.insert(ai.to_string(), deleg());
            p.register_member(d, m).unwrap();
        }
        p
    }

    fn no_root() -> &'static Path {
        Path::new("/joy-nonexistent-root-for-unit-test")
    }

    #[test]
    fn plan_absent_member_is_none() {
        let p = project_with("ai:claude@joy", &[]);
        assert!(plan_member_reset(&p, no_root(), "ai:qwen@joy", None).is_none());
    }

    #[test]
    fn plan_sole_delegator_removes_member() {
        let p = project_with("ai:claude@joy", &["op1@example.com"]);
        let plan =
            plan_member_reset(&p, no_root(), "ai:claude@joy", Some("op1@example.com")).unwrap();
        assert!(plan.drop_caller_delegation);
        assert_eq!(plan.other_delegators, 0);
        assert!(plan.remove_member);
        assert!(!plan.active_session);
    }

    #[test]
    fn plan_other_delegator_keeps_member() {
        let p = project_with("ai:claude@joy", &["op1@example.com", "op2@example.com"]);
        let plan =
            plan_member_reset(&p, no_root(), "ai:claude@joy", Some("op1@example.com")).unwrap();
        assert!(plan.drop_caller_delegation);
        assert_eq!(plan.other_delegators, 1);
        assert!(!plan.remove_member, "member kept while op2 still delegates");
    }

    #[test]
    fn plan_unknown_caller_keeps_delegated_member() {
        let p = project_with("ai:claude@joy", &["op1@example.com"]);
        let plan = plan_member_reset(&p, no_root(), "ai:claude@joy", None).unwrap();
        assert!(!plan.drop_caller_delegation);
        assert_eq!(plan.other_delegators, 1);
        assert!(!plan.remove_member);
    }

    #[test]
    fn plan_orphan_member_removed_even_with_unknown_caller() {
        // Member present but delegated by nobody (e.g. the delegation was
        // already removed): orphaned and removable, but only via the confirmed
        // path in reset(), never silently.
        let p = project_with("ai:claude@joy", &[]);
        let plan = plan_member_reset(&p, no_root(), "ai:claude@joy", None).unwrap();
        assert!(!plan.drop_caller_delegation);
        assert_eq!(plan.other_delegators, 0);
        assert!(plan.remove_member);
    }

    /// Write project defaults so the enforced global level is `level`.
    fn write_level_defaults(root: &Path, level: &str) {
        let joy = joy_core::store::joy_dir(root);
        std::fs::create_dir_all(&joy).unwrap();
        std::fs::write(
            joy.join("project.defaults.yaml"),
            format!("interaction-level:\n  default: {level}\n"),
        )
        .unwrap();
    }

    #[test]
    fn vibe_bash_permission_always_for_autonomous_member() {
        let tmp = tempfile::tempdir().unwrap();
        write_level_defaults(tmp.path(), "autonomous");
        let path = tmp.path().join("config.toml");
        let changed = ensure_vibe_bash_permission(tmp.path(), &path, "ai:test@joy").unwrap();
        assert!(changed);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[tools.bash]"));
        assert!(content.contains("permission = \"always\""));
    }

    #[test]
    fn vibe_bash_permission_adds_to_existing_unrelated_config() {
        let tmp = tempfile::tempdir().unwrap();
        write_level_defaults(tmp.path(), "autonomous");
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "[models]\ndefault = \"mistral-large\"\n").unwrap();
        let changed = ensure_vibe_bash_permission(tmp.path(), &path, "ai:test@joy").unwrap();
        assert!(changed);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("default = \"mistral-large\""));
        assert!(content.contains("permission = \"always\""));
    }

    #[test]
    fn vibe_bash_permission_enforces_level_over_hand_edit() {
        // A confirmed member's config hand-edited to `always` is pulled back
        // to `ask`: project.yaml is the authority (JOY-0222-4E).
        let tmp = tempfile::tempdir().unwrap();
        write_level_defaults(tmp.path(), "confirmed");
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "[tools.bash]\npermission = \"always\"\n").unwrap();
        let changed = ensure_vibe_bash_permission(tmp.path(), &path, "ai:test@joy").unwrap();
        assert!(changed);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("permission = \"ask\""));
        assert!(!content.contains("\"always\""));
    }

    #[test]
    fn vibe_bash_permission_noop_when_already_enforced() {
        let tmp = tempfile::tempdir().unwrap();
        write_level_defaults(tmp.path(), "confirmed");
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "[tools.bash]\npermission = \"ask\"\n").unwrap();
        let changed = ensure_vibe_bash_permission(tmp.path(), &path, "ai:test@joy").unwrap();
        assert!(!changed);
    }

    #[test]
    fn claude_settings_default_mode_follows_level() {
        let tmp = tempfile::tempdir().unwrap();
        write_level_defaults(tmp.path(), "autonomous");
        let mut lines = Vec::new();
        let mut report = |l: String| lines.push(l);
        update_claude_permissions(tmp.path(), "ai:test@joy", &mut report).unwrap();
        let content = std::fs::read_to_string(tmp.path().join(".claude/settings.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["permissions"]["defaultMode"], "bypassPermissions");

        write_level_defaults(tmp.path(), "proposing");
        update_claude_permissions(tmp.path(), "ai:test@joy", &mut report).unwrap();
        let content = std::fs::read_to_string(tmp.path().join(".claude/settings.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["permissions"]["defaultMode"], "plan");
        // The joy allowlist survives the enforcement.
        assert!(content.contains("Bash(joy *)"));
    }

    #[test]
    fn qwen_settings_approval_mode_follows_level() {
        let tmp = tempfile::tempdir().unwrap();
        write_level_defaults(tmp.path(), "confirmed");
        let mut lines = Vec::new();
        let mut report = |l: String| lines.push(l);
        update_qwen_permissions(tmp.path(), "ai:test@joy", &mut report).unwrap();
        let content = std::fs::read_to_string(tmp.path().join(".qwen/settings.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["approvalMode"], "auto-edit");
    }

    #[test]
    fn managed_block_carries_levels_and_tracks_changes() {
        let tmp = tempfile::tempdir().unwrap();
        write_level_defaults(tmp.path(), "proposing");
        let before = render_managed_block(tmp.path(), "ai:test@joy", true, "claude").unwrap();
        assert!(before.contains("## Interaction levels"));
        assert!(before.contains("Your interaction level: proposing"));
        assert!(before.contains("permission mode `plan`"));

        // A level change in the project data re-renders the block, which is
        // what makes is_tool_stale pick it up.
        write_level_defaults(tmp.path(), "autonomous");
        let after = render_managed_block(tmp.path(), "ai:test@joy", true, "claude").unwrap();
        assert_ne!(before, after);
        assert!(after.contains("permission mode `bypassPermissions`"));
    }

    #[test]
    fn update_gitignore_writes_full_set_regardless_of_configured_tools() {
        // Even with only Claude configured (or none), the block must contain
        // the full fixed set for all known tools, so machines never drift
        // (JOY-01AA-9E).
        let tmp = tempfile::tempdir().unwrap();
        update_gitignore(tmp.path(), &["claude"]).unwrap();

        let lines = read_block_lines(tmp.path());
        assert!(lines.iter().any(|l| l == ".joy/credentials.yaml"));
        for path in all_tool_paths() {
            assert!(
                lines.iter().any(|l| l == path),
                "managed block must contain {path}"
            );
        }
    }

    #[test]
    fn update_gitignore_full_set_even_with_no_tools_configured() {
        let tmp = tempfile::tempdir().unwrap();
        update_gitignore(tmp.path(), &[]).unwrap();

        let lines = read_block_lines(tmp.path());
        for path in all_tool_paths() {
            assert!(
                lines.iter().any(|l| l == path),
                "managed block must contain {path} even with no tools configured"
            );
        }
    }

    #[test]
    fn update_gitignore_is_idempotent() {
        // The per-invocation auto-sync must not churn the file: writing the
        // block twice yields byte-identical content (JOY-01AA-9E).
        let tmp = tempfile::tempdir().unwrap();
        update_gitignore(tmp.path(), &["claude"]).unwrap();
        let first = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        update_gitignore(tmp.path(), &["claude"]).unwrap();
        let second = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert_eq!(first, second, "repeated sync must not change .gitignore");
    }

    #[test]
    fn update_gitignore_does_not_resurrect_unrelated_lines() {
        // Foreign lines inside the block are not preserved across a rewrite;
        // Joy only manages its own known entries.
        let tmp = tempfile::tempdir().unwrap();
        seed_gitignore(
            tmp.path(),
            &[".joy/config.yaml", ".claude/", "user-injected-line.txt"],
        );

        update_gitignore(tmp.path(), &["claude"]).unwrap();

        let lines = read_block_lines(tmp.path());
        assert!(lines.iter().any(|l| l == ".claude/"));
        assert!(!lines.iter().any(|l| l == "user-injected-line.txt"));
    }

    #[test]
    fn untrack_gitignored_tool_files_untracks_committed_paths() {
        // A repo where AGENTS.md was committed before the ignore entry: after
        // untracking it is no longer tracked but stays on disk (JOY-019E-3A).
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "t"]);
        fs::write(root.join("AGENTS.md"), "committed").unwrap();
        fs::create_dir_all(root.join(".vibe")).unwrap();
        fs::write(root.join(".vibe/config.toml"), "x").unwrap();
        git(&["add", "AGENTS.md", ".vibe/config.toml"]);
        git(&["commit", "-q", "-m", "seed"]);
        assert!(git_path_is_tracked(root, "AGENTS.md"));
        assert!(git_path_is_tracked(root, ".vibe/"));

        untrack_gitignored_tool_files(root);

        assert!(!git_path_is_tracked(root, "AGENTS.md"));
        assert!(!git_path_is_tracked(root, ".vibe/"));
        // Files remain on disk.
        assert!(root.join("AGENTS.md").is_file());
        assert!(root.join(".vibe/config.toml").is_file());
    }

    #[test]
    fn untrack_gitignored_tool_files_is_noop_when_nothing_tracked() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["init", "-q"])
            .status()
            .unwrap();
        // No tool files tracked: must not panic or error.
        untrack_gitignored_tool_files(root);
        assert!(!git_path_is_tracked(root, "AGENTS.md"));
    }

    #[test]
    fn remove_legacy_ai_artifacts_removes_committed_and_preserves_runtime() {
        // Pre-ADR-024 layout committed into a repo: the legacy files are
        // removed from disk and the deletion is staged, while the current
        // runtime dirs .joy/ai/jobs and .joy/ai/agents survive.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "t"]);

        // Legacy artefacts.
        fs::create_dir_all(root.join(".joy/ai/instructions")).unwrap();
        fs::create_dir_all(root.join(".joy/ai/skills/joy")).unwrap();
        fs::create_dir_all(root.join(".joy/capabilities")).unwrap();
        fs::write(root.join(".joy/ai/instructions.md"), "old").unwrap();
        fs::write(root.join(".joy/ai/instructions/setup.md"), "old").unwrap();
        fs::write(root.join(".joy/ai/skills/joy/SKILL.md"), "old").unwrap();
        fs::write(root.join(".joy/capabilities/plan.md"), "old").unwrap();
        // Current runtime data that must be preserved.
        fs::create_dir_all(root.join(".joy/ai/jobs")).unwrap();
        fs::create_dir_all(root.join(".joy/ai/agents")).unwrap();
        fs::write(root.join(".joy/ai/jobs/j.yaml"), "id: 1").unwrap();
        fs::write(root.join(".joy/ai/agents/a.yaml"), "id: 2").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "seed"]);

        let removed = remove_legacy_ai_artifacts(root);

        assert_eq!(removed.len(), joy_core::init::LEGACY_AI_ARTIFACTS.len());
        // Legacy gone from disk and no longer tracked.
        assert!(!root.join(".joy/ai/instructions.md").exists());
        assert!(!root.join(".joy/ai/instructions").exists());
        assert!(!root.join(".joy/ai/skills").exists());
        assert!(!root.join(".joy/capabilities").exists());
        assert!(!git_path_is_tracked(root, ".joy/capabilities/"));
        // Runtime data preserved.
        assert!(root.join(".joy/ai/jobs/j.yaml").is_file());
        assert!(root.join(".joy/ai/agents/a.yaml").is_file());
        assert!(root.join(".joy/ai").is_dir());

        // Idempotent: a second pass finds nothing.
        assert!(remove_legacy_ai_artifacts(root).is_empty());
    }

    #[test]
    fn remove_legacy_ai_artifacts_prunes_empty_ai_dir() {
        // When no jobs/agents remain, an emptied .joy/ai/ is pruned too.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join(".joy/ai/skills/joy")).unwrap();
        fs::write(root.join(".joy/ai/skills/joy/SKILL.md"), "old").unwrap();

        let removed = remove_legacy_ai_artifacts(root);

        assert!(removed.contains(&".joy/ai/skills".to_string()));
        assert!(!root.join(".joy/ai").exists());
    }

    #[test]
    fn update_with_joy_block_creates_agents_md() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("AGENTS.md");
        let changed = update_with_joy_block(tmp.path(), &path, "hello world").unwrap();
        assert!(changed);
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.starts_with(JOY_BLOCK_START));
        assert!(content.contains("hello world"));
        assert!(content.contains(JOY_BLOCK_END));
    }

    #[test]
    fn update_with_joy_block_preserves_user_content_above_and_below() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("AGENTS.md");
        fs::write(
            &path,
            format!(
                "user header\n\n{}\nold\n{}\n\nuser footer\n",
                JOY_BLOCK_START, JOY_BLOCK_END
            ),
        )
        .unwrap();
        update_with_joy_block(tmp.path(), &path, "new content").unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("user header"));
        assert!(content.contains("new content"));
        assert!(!content.contains("old"));
        assert!(content.trim_end().ends_with("user footer"));
    }
}
