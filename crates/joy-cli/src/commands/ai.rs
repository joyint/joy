// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use joy_core::ai_templates;

use crate::color;

static QUIET: AtomicBool = AtomicBool::new(false);

macro_rules! qprintln {
    ($($arg:tt)*) => {
        if !QUIET.load(std::sync::atomic::Ordering::Relaxed) {
            println!($($arg)*);
        }
    };
}

const VISION_TEMPLATE: &str = include_str!("../../../../templates/docs/vision/README.md");
const ARCHITECTURE_TEMPLATE: &str =
    include_str!("../../../../templates/docs/architecture/README.md");
const CONTRIBUTING_TEMPLATE: &str = include_str!("../../../../templates/docs/CONTRIBUTING.md");

const JOY_VERSION: &str = env!("CARGO_PKG_VERSION");

const JOY_BLOCK_START: &str = "<!-- joy:start -->";
const JOY_BLOCK_END: &str = "<!-- joy:end -->";

#[derive(clap::Args)]
pub struct AiArgs {
    #[command(subcommand)]
    command: AiCommand,
}

#[derive(clap::Subcommand)]
enum AiCommand {
    /// Set up AI tool integration for this project
    Setup,
    /// Check if AI templates are up to date, update if needed
    Check,
    /// Remove AI tool configurations from this project
    Reset(ResetArgs),
}

#[derive(clap::Args)]
struct ResetArgs {
    /// Only reset a specific tool (claude, qwen, vibe, copilot)
    #[arg(long)]
    tool: Option<String>,

    /// Skip confirmation prompt
    #[arg(long, short)]
    force: bool,
}

pub fn run(args: AiArgs) -> anyhow::Result<()> {
    match args.command {
        AiCommand::Setup => setup(),
        AiCommand::Check => check(),
        AiCommand::Reset(a) => reset(a),
    }
}

fn setup() -> anyhow::Result<()> {
    let root = joy_core::store::find_project_root(&std::env::current_dir()?)
        .ok_or_else(|| anyhow::anyhow!("No Joy project found (run `joy init` first)"))?;

    println!("{}", color::header("AI Setup"));
    println!();

    // Ensure project.defaults.yaml exists
    joy_core::embedded::sync_files(&root, joy_core::init::PROJECT_FILES)?;

    check_docs(&root)?;
    let configured_tools = configure_tools(&root)?;
    update_gitignore(&root, &configured_tools)?;
    check_nested_projects(&root)?;

    let msg = format!(
        "AI integration complete -- {}",
        color::plural(configured_tools.len(), "tool")
    );
    println!("{}", color::footer(&msg));
    Ok(())
}

fn check_docs(root: &Path) -> anyhow::Result<()> {
    println!("{}", color::section("Documentation"));

    let docs = [
        (
            "docs/dev/vision/README.md",
            "product goals and design decisions",
            VISION_TEMPLATE,
        ),
        (
            "docs/dev/architecture/README.md",
            "technical stack and structure",
            ARCHITECTURE_TEMPLATE,
        ),
        (
            "CONTRIBUTING.md",
            "coding conventions and commit messages",
            CONTRIBUTING_TEMPLATE,
        ),
    ];

    let mut all_found = true;
    for (path, purpose, template) in &docs {
        let full = root.join(path);
        if full.is_file() {
            println!("  {}{}", color::check_mark(), path);
        } else {
            println!("  {}{}", color::cross_mark(), color::warning(path));
            let name = path.rsplit('/').next().unwrap_or(path);
            print!(
                "    {} helps AI understand your {}. Create template? [Y/n] ",
                name, purpose
            );
            std::io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let trimmed = input.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("y") {
                if let Some(parent) = full.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&full, template)?;
                println!(
                    "    {}Created {} (template -- your AI tool will help fill it in)",
                    color::check_mark(),
                    path
                );
            }
            all_found = false;
        }
    }

    if !all_found {
        println!(
            "\n  {}Your AI tool will offer to fill in empty templates on first use.",
            color::warn_mark()
        );
    }
    println!();

    Ok(())
}

fn check() -> anyhow::Result<()> {
    let root = joy_core::store::find_project_root(&std::env::current_dir()?)
        .ok_or_else(|| anyhow::anyhow!("No Joy project found (run `joy init` first)"))?;

    println!("{}", color::header("AI Status"));
    println!();

    // AI Tools -- re-render expected output and compare against on-disk
    println!("{}", color::section("AI Tools"));
    let tool_checks: &[(&str, &str)] = &[
        ("Claude Code", "claude"),
        ("Qwen Code", "qwen"),
        ("Mistral Vibe", "vibe"),
        ("GitHub Copilot", "copilot"),
    ];

    let mut configured_count = 0;
    let mut has_issues = false;
    for (name, id) in tool_checks {
        let installed = match *id {
            "claude" => which("claude"),
            "qwen" => which("qwen") || which("qwen-code"),
            "vibe" => which("vibe"),
            "copilot" => which("copilot") || which("gh"),
            _ => false,
        };
        let configured = is_tool_configured(&root, id);
        if configured {
            configured_count += 1;
            // Check if generated files are up-to-date by re-rendering and comparing
            let member_id = format!("ai:{id}@joy");
            let stale = check_tool_files(&root, id, &member_id).unwrap_or(true);
            if stale {
                has_issues = true;
                println!(
                    "  {}{:<24} {}",
                    color::warn_mark(),
                    name,
                    color::warning("outdated")
                );
            } else {
                println!(
                    "  {}{:<24} {}",
                    color::check_mark(),
                    name,
                    color::inactive("up to date")
                );
            }
        } else if installed {
            println!(
                "  {}{:<24} {}",
                color::warn_mark(),
                name,
                color::warning("installed, not configured")
            );
        } else {
            println!("    {:<24} {}", name, color::inactive("not installed"));
        }
    }

    println!();
    let msg = format!(
        "{} · {}",
        if has_issues {
            format!("files need update -- run {}", color::label("joy ai setup"))
        } else {
            "all up to date".to_string()
        },
        color::plural(configured_count, "tool")
    );
    println!("{}", color::footer(&msg));

    if has_issues {
        std::process::exit(2);
    }
    Ok(())
}

/// Check if a tool's generated files are up to date by comparing version headers.
/// Returns true if any file is stale (version mismatch or missing).
fn check_tool_files(root: &Path, tool: &str, _member_id: &str) -> anyhow::Result<bool> {
    // Collect all files that should have a version header
    let mut files_to_check: Vec<std::path::PathBuf> = Vec::new();

    // SKILL.md (all tools except copilot)
    match tool {
        "claude" => files_to_check.push(root.join(".claude/skills/joy/SKILL.md")),
        "qwen" => files_to_check.push(root.join(".qwen/skills/joy/SKILL.md")),
        "vibe" => files_to_check.push(root.join(".vibe/skills/joy/SKILL.md")),
        _ => {}
    }

    // Instructions file (joy-block contains version header)
    match tool {
        "claude" => files_to_check.push(root.join(".claude/CLAUDE.md")),
        "qwen" => files_to_check.push(root.join(".qwen/QWEN.md")),
        "copilot" => {
            files_to_check.push(root.join(".github/copilot-instructions.md"));
            files_to_check.push(root.join(".github/prompts/joy.prompt.md"));
        }
        _ => {}
    }

    // Agent files
    let agents_dir = match tool {
        "claude" => Some(root.join(".claude/agents")),
        "qwen" => Some(root.join(".qwen/agents")),
        "vibe" => Some(root.join(".vibe/agents")),
        "copilot" => Some(root.join(".github/agents")),
        _ => None,
    };
    if let Some(dir) = agents_dir {
        if dir.is_dir() {
            for entry in fs::read_dir(&dir)?.filter_map(|e| e.ok()) {
                files_to_check.push(entry.path());
            }
        }
    }

    for path in &files_to_check {
        if !path.is_file() {
            return Ok(true); // missing = stale
        }
        let content = fs::read_to_string(path)?;
        match ai_templates::extract_version(&content) {
            Some(version) if version == JOY_VERSION => {} // up to date
            _ => return Ok(true),                         // stale or no header
        }
    }

    Ok(false) // all up to date
}

fn reset(args: ResetArgs) -> anyhow::Result<()> {
    let root = joy_core::store::find_project_root(&std::env::current_dir()?)
        .ok_or_else(|| anyhow::anyhow!("No Joy project found (run `joy init` first)"))?;

    let all_tools: &[(&str, &str, &[&str])] = &[
        ("Claude Code", "claude", &[".claude/"]),
        ("Qwen Code", "qwen", &[".qwen/"]),
        ("Mistral Vibe", "vibe", &[".vibe/"]),
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

    let tools: Vec<_> = if let Some(ref filter) = args.tool {
        let found = all_tools.iter().find(|(_, id, _)| id == filter);
        match found {
            Some(t) => vec![*t],
            None => {
                let valid: Vec<_> = all_tools.iter().map(|(_, id, _)| *id).collect();
                anyhow::bail!("unknown tool: {filter}\nknown tools: {}", valid.join(", "));
            }
        }
    } else {
        all_tools.to_vec()
    };

    // Collect what exists
    let mut to_remove: Vec<(&str, &str)> = Vec::new();
    for (name, _, paths) in &tools {
        for path in *paths {
            let full = root.join(path);
            if full.exists() {
                to_remove.push((name, path));
            }
        }
    }

    if to_remove.is_empty() {
        // No files to remove, but check for orphaned members
        let project_path = joy_core::store::joy_dir(&root).join(joy_core::store::PROJECT_FILE);
        if let Ok(mut project) =
            joy_core::store::read_yaml::<joy_core::model::Project>(&project_path)
        {
            let mut cleaned = false;
            for (_, id, _) in &tools {
                let member_id = format!("ai:{id}@joy");
                if project.members.remove(&member_id).is_some() {
                    println!(
                        "  {}{:<24} orphaned member removed",
                        color::check_mark(),
                        member_id
                    );
                    cleaned = true;
                }
            }
            if cleaned {
                joy_core::store::write_yaml_preserve(&project_path, &project)?;
            } else {
                println!("{}No AI tool configurations found.", color::check_mark());
            }
        } else {
            println!("{}No AI tool configurations found.", color::check_mark());
        }
        return Ok(());
    }

    println!("{}", color::header("AI Reset"));
    println!();
    println!("Will remove:");
    for (name, path) in &to_remove {
        println!("  {}{:<24} {}", color::cross_mark(), name, path);
    }

    if !args.force {
        println!();
        print!("Proceed? [y/N] ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let trimmed = input.trim();
        if !trimmed.eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(());
        }
    }

    for (name, path) in &to_remove {
        let full = root.join(path);
        if full.is_dir() {
            fs::remove_dir_all(&full)?;
        } else {
            fs::remove_file(&full)?;
        }
        println!("  {}{:<24} removed", color::check_mark(), name);
    }

    // Remove AI members from project.yaml for reset tools
    let project_path = joy_core::store::joy_dir(&root).join(joy_core::store::PROJECT_FILE);
    if let Ok(mut project) = joy_core::store::read_yaml::<joy_core::model::Project>(&project_path) {
        let mut project_changed = false;
        for (_, id, paths) in &tools {
            let was_removed = paths
                .iter()
                .any(|p| to_remove.iter().any(|(_, tp)| tp == p));
            if was_removed {
                let member_id = format!("ai:{id}@joy");
                if project.members.remove(&member_id).is_some() {
                    println!("  {}{:<24} member removed", color::check_mark(), member_id);
                    project_changed = true;
                    // Remove AI member's session if one exists
                    if let Ok(project_id) = joy_core::auth::session::project_id(&root) {
                        let _ = joy_core::auth::session::remove_session(&project_id, &member_id);
                    }
                    // Remove all ai_tokens entries for this AI member from all human members
                    for (_, m) in project.members.iter_mut() {
                        m.ai_tokens.remove(&member_id);
                    }
                }
            }
        }
        if project_changed {
            joy_core::store::write_yaml_preserve(&project_path, &project)?;
        }
    }

    // If no AI tools remain, update gitignore to remove tool entries
    let any_remaining = all_tools
        .iter()
        .any(|(_, id, _)| is_tool_configured(&root, id));
    if !any_remaining {
        joy_core::init::update_gitignore_block(&root, joy_core::init::GITIGNORE_BASE_ENTRIES)?;
    }

    let count = tools
        .iter()
        .filter(|(_, _, paths)| {
            paths
                .iter()
                .any(|p| to_remove.iter().any(|(_, tp)| tp == p))
        })
        .count();
    println!(
        "{}",
        color::footer(&format!("{} reset", color::plural(count, "tool")))
    );
    Ok(())
}

type ToolEntry = (
    &'static str,
    &'static str,
    fn(bool) -> bool,
    fn(&Path, &str) -> anyhow::Result<bool>,
);

/// Returns the list of configured tool IDs.
fn configure_tools(root: &Path) -> anyhow::Result<Vec<&'static str>> {
    println!("{}", color::section("AI Tools"));

    let mut configured_tools: Vec<&'static str> = Vec::new();

    let tools: &[ToolEntry] = &[
        (
            "Claude Code",
            "claude",
            |_| which("claude"),
            configure_claude,
        ),
        (
            "Qwen Code",
            "qwen",
            |_| which("qwen") || which("qwen-code"),
            configure_qwen,
        ),
        ("Mistral Vibe", "vibe", |_| which("vibe"), configure_vibe),
        (
            "GitHub Copilot",
            "copilot",
            |_| which("copilot") || which("gh"),
            configure_copilot,
        ),
    ];

    // Load project for member registration
    let project_path = joy_core::store::joy_dir(root).join(joy_core::store::PROJECT_FILE);
    let mut project: joy_core::model::Project = joy_core::store::read_yaml(&project_path)?;
    let mut project_changed = false;

    for (name, id, detect, configure) in tools {
        if !detect(false) {
            continue;
        }
        let already = is_tool_configured(root, id);
        let member_id = format!("ai:{id}@joy");
        let mut configured = false;
        if already {
            QUIET.store(true, std::sync::atomic::Ordering::Relaxed);
            let changed = configure(root, &member_id)?;
            QUIET.store(false, std::sync::atomic::Ordering::Relaxed);
            let status = if changed {
                color::success("updated")
            } else {
                color::inactive("up to date")
            };
            println!("  {}{:<24} {}", color::check_mark(), name, status);
            configured = true;
            configured_tools.push(*id);
        } else {
            print!("  {}{:<24} configure? [Y/n] ", color::warn_mark(), name);
            if confirm_default_yes()? {
                configure(root, &member_id)?;
                configured = true;
                configured_tools.push(*id);
            }
        }

        // Register as AI member only if tool was actually configured
        if configured && !project.members.contains_key(&member_id) {
            let ai_defaults = joy_core::store::load_ai_defaults(root);
            let ai_caps = if ai_defaults.capabilities.is_empty() {
                joy_core::model::item::Capability::work_capabilities()
            } else {
                ai_defaults.capabilities.clone()
            };
            project.members.insert(
                member_id.clone(),
                joy_core::model::project::Member::new(
                    joy_core::model::project::MemberCapabilities::Specific({
                        use joy_core::model::project::CapabilityConfig;
                        let mut map = std::collections::BTreeMap::new();
                        for cap in ai_caps {
                            map.insert(cap, CapabilityConfig::default());
                        }
                        map
                    }),
                ),
            );
            project_changed = true;
            println!(
                "  {}{:<24} {}",
                color::check_mark(),
                member_id,
                color::success("registered as member")
            );
        }
    }

    if project_changed {
        joy_core::store::write_yaml_preserve(&project_path, &project)?;
    }

    if configured_tools.is_empty() {
        println!("  {}No supported AI tools detected.", color::warn_mark());
        println!(
            "  {}",
            color::inactive("Supported: Claude Code, Qwen Code, Mistral Vibe, GitHub Copilot")
        );
        println!(
            "  {}",
            color::inactive("Install one and re-run `joy ai setup`.")
        );
    }

    println!();
    Ok(configured_tools)
}

/// Render the managed block (identity + instructions with workflow) for a tool's instruction file.
fn render_managed_block(member_id: &str, has_skill: bool) -> anyhow::Result<String> {
    let workflow = ai_templates::load_workflow()?;
    let joy_block = ai_templates::render_joy_block(member_id, has_skill, JOY_VERSION)?;
    let instructions = ai_templates::render_instructions(&workflow)?;
    Ok(format!("{}\n\n{}", joy_block, instructions))
}

/// Render SKILL.md with workflow context.
fn render_skill() -> anyhow::Result<String> {
    let workflow = ai_templates::load_workflow()?;
    ai_templates::render_skill(&workflow, JOY_VERSION).map_err(Into::into)
}

/// Remove and recreate Joy-managed subdirectories for a tool.
/// Preserves user-owned files (instruction files, settings.json).
fn clean_managed_dirs(root: &Path, dirs: &[&str]) {
    for dir in dirs {
        let path = root.join(dir);
        if path.is_dir() {
            let _ = fs::remove_dir_all(&path);
        }
    }
}

/// Generate agent files for a tool into the given directory.
fn generate_agents(root: &Path, tool: &str, agents_dir: &str) -> anyhow::Result<bool> {
    let workflow = ai_templates::load_workflow()?;
    let agents = ai_templates::load_agents()?;
    let mut changed = false;

    for agent in &agents {
        if !ai_templates::agent_applicable_to_tool(agent, tool) {
            continue;
        }
        if let Some(filename) = ai_templates::agent_filename(agent, tool) {
            let content = ai_templates::render_agent(agent, &workflow, tool, JOY_VERSION)?;
            let path = root.join(agents_dir).join(&filename);
            changed |= write_if_changed(&path, &content)?;
            qprintln!("    {}{}/{}", color::check_mark(), agents_dir, filename);
        }
    }
    Ok(changed)
}

fn configure_claude(root: &Path, member_id: &str) -> anyhow::Result<bool> {
    let claude_dir = root.join(".claude");
    fs::create_dir_all(&claude_dir)?;
    clean_managed_dirs(root, &[".claude/agents", ".claude/skills/joy"]);
    let mut changed = false;

    let claude_md = claude_dir.join("CLAUDE.md");
    changed |= update_with_joy_block(&claude_md, &render_managed_block(member_id, true)?)?;
    qprintln!("    {}.claude/CLAUDE.md", color::check_mark());

    let skill_path = claude_dir.join("skills/joy/SKILL.md");
    changed |= write_if_changed(&skill_path, &render_skill()?)?;
    qprintln!("    {}.claude/skills/joy/SKILL.md", color::check_mark());

    let setup_path = claude_dir.join("skills/joy/setup.md");
    changed |= write_if_changed(&setup_path, ai_templates::setup_instructions())?;
    qprintln!("    {}.claude/skills/joy/setup.md", color::check_mark());

    changed |= generate_agents(root, "claude", ".claude/agents")?;
    changed |= update_claude_permissions(root)?;

    Ok(changed)
}

fn update_claude_permissions(root: &Path) -> anyhow::Result<bool> {
    let settings_path = root.join(".claude/settings.json");
    let joy_permission = "Bash(joy *)";
    let jot_permission = "Bash(jot *)";

    let mut settings: serde_json::Value = if settings_path.is_file() {
        let content = fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

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

    for perm in [joy_permission, jot_permission] {
        if !allow_arr.iter().any(|v| v.as_str() == Some(perm)) {
            allow_arr.push(serde_json::json!(perm));
        }
    }

    let json = serde_json::to_string_pretty(&settings)?;
    let changed = write_if_changed(&settings_path, &format!("{json}\n"))?;
    qprintln!("    {}.claude/settings.json", color::check_mark());

    Ok(changed)
}

fn configure_qwen(root: &Path, member_id: &str) -> anyhow::Result<bool> {
    let qwen_dir = root.join(".qwen");
    fs::create_dir_all(&qwen_dir)?;
    clean_managed_dirs(root, &[".qwen/agents", ".qwen/skills/joy"]);
    let mut changed = false;

    let qwen_md = qwen_dir.join("QWEN.md");
    changed |= update_with_joy_block(&qwen_md, &render_managed_block(member_id, true)?)?;
    qprintln!("    {}.qwen/QWEN.md", color::check_mark());

    let skill_path = qwen_dir.join("skills/joy/SKILL.md");
    changed |= write_if_changed(&skill_path, &render_skill()?)?;
    qprintln!("    {}.qwen/skills/joy/SKILL.md", color::check_mark());

    let setup_path = qwen_dir.join("skills/joy/setup.md");
    changed |= write_if_changed(&setup_path, ai_templates::setup_instructions())?;
    qprintln!("    {}.qwen/skills/joy/setup.md", color::check_mark());

    changed |= generate_agents(root, "qwen", ".qwen/agents")?;
    changed |= update_qwen_permissions(root)?;

    Ok(changed)
}

fn update_qwen_permissions(root: &Path) -> anyhow::Result<bool> {
    let settings_path = root.join(".qwen/settings.json");

    let mut settings: serde_json::Value = if settings_path.is_file() {
        let content = fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Ensure tools.allowed array exists and contains joy/jot
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

    for perm in ["run_shell_command(joy)", "run_shell_command(jot)"] {
        if !allowed_arr.iter().any(|v| v.as_str() == Some(perm)) {
            allowed_arr.push(serde_json::json!(perm));
        }
    }

    let json = serde_json::to_string_pretty(&settings)?;
    let changed = write_if_changed(&settings_path, &format!("{json}\n"))?;
    qprintln!("    {}.qwen/settings.json", color::check_mark());

    Ok(changed)
}

fn configure_vibe(root: &Path, _member_id: &str) -> anyhow::Result<bool> {
    let vibe_dir = root.join(".vibe");
    fs::create_dir_all(&vibe_dir)?;
    clean_managed_dirs(root, &[".vibe/agents", ".vibe/skills/joy"]);
    let mut changed = false;

    let skill_path = vibe_dir.join("skills/joy/SKILL.md");
    changed |= write_if_changed(&skill_path, &render_skill()?)?;
    qprintln!("    {}.vibe/skills/joy/SKILL.md", color::check_mark());

    let setup_path = vibe_dir.join("skills/joy/setup.md");
    changed |= write_if_changed(&setup_path, ai_templates::setup_instructions())?;
    qprintln!("    {}.vibe/skills/joy/setup.md", color::check_mark());

    changed |= generate_agents(root, "vibe", ".vibe/agents")?;

    qprintln!(
        "    {}",
        color::inactive("Note: set [tools.bash] permission = \"always\" in .vibe/config.toml")
    );

    Ok(changed)
}

fn configure_copilot(root: &Path, member_id: &str) -> anyhow::Result<bool> {
    let github_dir = root.join(".github");
    fs::create_dir_all(&github_dir)?;
    clean_managed_dirs(root, &[".github/agents", ".github/prompts"]);
    let mut changed = false;

    let instructions_md = github_dir.join("copilot-instructions.md");
    changed |= update_with_joy_block(&instructions_md, &render_managed_block(member_id, false)?)?;
    qprintln!("    {}.github/copilot-instructions.md", color::check_mark());

    // Copilot skill wrapper
    let workflow = ai_templates::load_workflow()?;
    let prompt = ai_templates::render_copilot_prompt(&workflow, JOY_VERSION)?;
    let prompt_path = github_dir.join("prompts/joy.prompt.md");
    changed |= write_if_changed(&prompt_path, &prompt)?;
    qprintln!("    {}.github/prompts/joy.prompt.md", color::check_mark());

    changed |= generate_agents(root, "copilot", ".github/agents")?;
    changed |= update_copilot_permissions(root)?;

    Ok(changed)
}

fn update_copilot_permissions(root: &Path) -> anyhow::Result<bool> {
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

    for perm in ["shell(joy:*)", "shell(jot:*)"] {
        if !allow_arr.iter().any(|v| v.as_str() == Some(perm)) {
            allow_arr.push(serde_json::json!(perm));
        }
    }

    let json = serde_json::to_string_pretty(&settings)?;
    let changed = write_if_changed(&settings_path, &format!("{json}\n"))?;
    qprintln!("    {}.github/copilot/settings.json", color::check_mark());

    Ok(changed)
}

/// Write content to a file only if it differs (hash comparison).
/// Returns true if the file was changed.
fn write_if_changed(path: &Path, content: &str) -> anyhow::Result<bool> {
    use std::hash::{DefaultHasher, Hash, Hasher};

    if path.is_file() {
        let existing = fs::read_to_string(path)?;
        let mut h1 = DefaultHasher::new();
        existing.hash(&mut h1);
        let mut h2 = DefaultHasher::new();
        content.hash(&mut h2);
        if h1.finish() == h2.finish() {
            return Ok(false);
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(true)
}

fn update_with_joy_block(path: &Path, content: &str) -> anyhow::Result<bool> {
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

    write_if_changed(path, &new_content)
}

fn confirm_default_yes() -> anyhow::Result<bool> {
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    Ok(trimmed.is_empty() || trimmed.eq_ignore_ascii_case("y"))
}

fn which(binary: &str) -> bool {
    std::process::Command::new("which")
        .arg(binary)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Gitignore entries per AI tool.
const TOOL_GITIGNORE_ENTRIES: &[(&str, &[(&str, &str)])] = &[
    ("claude", &[(".claude/", "Claude Code")]),
    ("qwen", &[(".qwen/", "Qwen Code")]),
    ("vibe", &[(".vibe/", "Mistral Vibe")]),
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

fn update_gitignore(root: &Path, configured_tools: &[&str]) -> anyhow::Result<()> {
    use joy_core::init::GITIGNORE_BASE_ENTRIES;

    let mut entries: Vec<(&str, &str)> = GITIGNORE_BASE_ENTRIES.to_vec();

    for (tool_id, tool_entries) in TOOL_GITIGNORE_ENTRIES {
        if configured_tools.contains(tool_id) {
            entries.extend_from_slice(tool_entries);
        }
    }

    joy_core::init::update_gitignore_block(root, &entries)?;
    Ok(())
}

/// Scan subdirectories (max 2 levels) for nested Joy projects that lack AI tool config.
fn check_nested_projects(root: &Path) -> anyhow::Result<()> {
    let mut unconfigured: Vec<String> = Vec::new();

    // Collect installed tools to check against
    let tools: Vec<&str> = ["claude", "qwen", "vibe", "copilot"]
        .iter()
        .copied()
        .filter(|t| {
            which(t) || (*t == "qwen" && which("qwen-code")) || (*t == "copilot" && which("gh"))
        })
        .collect();

    if tools.is_empty() {
        return Ok(());
    }

    // Scan 2 levels deep for .joy/project.yaml
    for entry in fs::read_dir(root)?.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_dir()
            || path
                .file_name()
                .is_some_and(|n| n.to_str().is_some_and(|s| s.starts_with('.')))
        {
            continue;
        }
        check_nested_at(&path, root, &tools, &mut unconfigured);
        // Level 2
        if let Ok(sub_entries) = fs::read_dir(&path) {
            for sub_entry in sub_entries.filter_map(|e| e.ok()) {
                let sub_path = sub_entry.path();
                if !sub_path.is_dir()
                    || sub_path
                        .file_name()
                        .is_some_and(|n| n.to_str().is_some_and(|s| s.starts_with('.')))
                {
                    continue;
                }
                check_nested_at(&sub_path, root, &tools, &mut unconfigured);
            }
        }
    }

    if !unconfigured.is_empty() {
        println!("{}", color::section("Nested Projects"));
        for path in &unconfigured {
            println!("  {}{}/", color::warn_mark(), path);
        }
        println!(
            "  {}",
            color::inactive("Permissions are per-project. Run `joy ai setup` in each.")
        );
        println!();
    }

    Ok(())
}

fn check_nested_at(dir: &Path, root: &Path, tools: &[&str], unconfigured: &mut Vec<String>) {
    let project_yaml = dir.join(".joy/project.yaml");
    if !project_yaml.is_file() {
        return;
    }
    // At least one installed tool must be unconfigured here
    let any_configured = tools.iter().any(|t| is_tool_configured(dir, t));
    if !any_configured {
        let relative = dir
            .strip_prefix(root)
            .unwrap_or(dir)
            .to_string_lossy()
            .to_string();
        unconfigured.push(relative);
    }
}

fn is_tool_configured(root: &Path, tool: &str) -> bool {
    match tool {
        "claude" => root.join(".claude/CLAUDE.md").is_file(),
        "qwen" => root.join(".qwen/QWEN.md").is_file(),
        "vibe" => root.join(".vibe/skills/joy/SKILL.md").is_file(),
        "copilot" => root.join(".github/copilot-instructions.md").is_file(),
        _ => false,
    }
}
