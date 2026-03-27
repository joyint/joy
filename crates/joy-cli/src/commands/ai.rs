// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use joy_core::embedded::{self, EmbeddedFile, FileStatus};

use crate::color;

static QUIET: AtomicBool = AtomicBool::new(false);

macro_rules! qprintln {
    ($($arg:tt)*) => {
        if !QUIET.load(std::sync::atomic::Ordering::Relaxed) {
            println!($($arg)*);
        }
    };
}

const INSTRUCTIONS_TEMPLATE: &str = include_str!("../../../../data/ai/instructions.md");
const SETUP_INSTRUCTIONS: &str = include_str!("../../../../data/ai/instructions/setup.md");
const SKILL_TEMPLATE: &str = include_str!("../../../../data/ai/skills/joy/SKILL.md");
const VISION_TEMPLATE: &str = include_str!("../../../../data/ai/templates/Vision.md");
const ARCHITECTURE_TEMPLATE: &str = include_str!("../../../../data/ai/templates/Architecture.md");
const CONTRIBUTING_TEMPLATE: &str = include_str!("../../../../data/ai/templates/CONTRIBUTING.md");
const JOY_BLOCK_TEMPLATE: &str = include_str!("../../../../data/ai/joy-block.md");

// Capability definitions
const CAP_CONCEIVE: &str = include_str!("../../../../data/capabilities/conceive.md");
const CAP_PLAN: &str = include_str!("../../../../data/capabilities/plan.md");
const CAP_DESIGN: &str = include_str!("../../../../data/capabilities/design.md");
const CAP_IMPLEMENT: &str = include_str!("../../../../data/capabilities/implement.md");
const CAP_TEST: &str = include_str!("../../../../data/capabilities/test.md");
const CAP_REVIEW: &str = include_str!("../../../../data/capabilities/review.md");
const CAP_DOCUMENT: &str = include_str!("../../../../data/capabilities/document.md");
const CAP_CREATE: &str = include_str!("../../../../data/capabilities/create.md");
const CAP_ASSIGN: &str = include_str!("../../../../data/capabilities/assign.md");
const CAP_MANAGE: &str = include_str!("../../../../data/capabilities/manage.md");
const CAP_DELETE: &str = include_str!("../../../../data/capabilities/delete.md");

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

    check_docs(&root)?;
    copy_templates(&root)?;
    let tool_count = configure_tools(&root)?;
    check_nested_projects(&root)?;

    let msg = format!(
        "AI integration complete -- {} tool(s) configured",
        tool_count
    );
    println!("{}", color::footer(&msg));
    Ok(())
}

fn check_docs(root: &Path) -> anyhow::Result<()> {
    println!("{}", color::section("Documentation"));

    let docs = [
        (
            "docs/dev/Vision.md",
            "product goals and design decisions",
            VISION_TEMPLATE,
        ),
        (
            "docs/dev/Architecture.md",
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

const AI_FILES: &[EmbeddedFile] = &[
    EmbeddedFile {
        content: INSTRUCTIONS_TEMPLATE,
        target: "ai/instructions.md",
        executable: false,
    },
    EmbeddedFile {
        content: SETUP_INSTRUCTIONS,
        target: "ai/instructions/setup.md",
        executable: false,
    },
    EmbeddedFile {
        content: SKILL_TEMPLATE,
        target: "ai/skills/joy/SKILL.md",
        executable: false,
    },
    // Capability definitions
    EmbeddedFile {
        content: CAP_CONCEIVE,
        target: "capabilities/conceive.md",
        executable: false,
    },
    EmbeddedFile {
        content: CAP_PLAN,
        target: "capabilities/plan.md",
        executable: false,
    },
    EmbeddedFile {
        content: CAP_DESIGN,
        target: "capabilities/design.md",
        executable: false,
    },
    EmbeddedFile {
        content: CAP_IMPLEMENT,
        target: "capabilities/implement.md",
        executable: false,
    },
    EmbeddedFile {
        content: CAP_TEST,
        target: "capabilities/test.md",
        executable: false,
    },
    EmbeddedFile {
        content: CAP_REVIEW,
        target: "capabilities/review.md",
        executable: false,
    },
    EmbeddedFile {
        content: CAP_DOCUMENT,
        target: "capabilities/document.md",
        executable: false,
    },
    EmbeddedFile {
        content: CAP_CREATE,
        target: "capabilities/create.md",
        executable: false,
    },
    EmbeddedFile {
        content: CAP_ASSIGN,
        target: "capabilities/assign.md",
        executable: false,
    },
    EmbeddedFile {
        content: CAP_MANAGE,
        target: "capabilities/manage.md",
        executable: false,
    },
    EmbeddedFile {
        content: CAP_DELETE,
        target: "capabilities/delete.md",
        executable: false,
    },
];

fn check() -> anyhow::Result<()> {
    let root = joy_core::store::find_project_root(&std::env::current_dir()?)
        .ok_or_else(|| anyhow::anyhow!("No Joy project found (run `joy init` first)"))?;

    let diffs = embedded::diff_files(&root, AI_FILES)?;
    let issues: Vec<_> = diffs
        .iter()
        .filter(|(_, s)| *s != FileStatus::UpToDate)
        .collect();

    if issues.is_empty() {
        println!("{}AI templates up to date.", color::check_mark());
        std::process::exit(0);
    }

    for (path, status) in &issues {
        let label = match status {
            FileStatus::Outdated => color::warning("outdated"),
            FileStatus::Missing => color::danger("missing"),
            FileStatus::UpToDate => unreachable!(),
        };
        println!("  {} .joy/{}", label, path);
    }
    println!("\nRun {} to update.", color::label("joy ai setup"));
    std::process::exit(2);
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
            &[".github/copilot-instructions.md"],
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
                joy_core::store::write_yaml(&project_path, &project)?;
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
                }
            }
        }
        if project_changed {
            joy_core::store::write_yaml(&project_path, &project)?;
        }
    }

    let count = tools
        .iter()
        .filter(|(_, _, paths)| {
            paths
                .iter()
                .any(|p| to_remove.iter().any(|(_, tp)| tp == p))
        })
        .count();
    println!("{}", color::footer(&format!("{} tool(s) reset", count)));
    Ok(())
}

fn copy_templates(root: &Path) -> anyhow::Result<()> {
    println!("{}", color::section("Templates"));

    let actions = embedded::sync_files(root, AI_FILES)?;

    for action in &actions {
        let status = if action.action == "up to date" {
            color::inactive(action.action)
        } else if action.action == "installed" || action.action == "updated" {
            color::success(action.action)
        } else {
            action.action.to_string()
        };
        println!(
            "  {}{:<32} {}",
            color::check_mark(),
            format!(".joy/{}", action.target),
            status
        );
    }

    println!();
    Ok(())
}

type ToolEntry = (
    &'static str,
    &'static str,
    fn(bool) -> bool,
    fn(&Path, &str) -> anyhow::Result<bool>,
);

fn configure_tools(root: &Path) -> anyhow::Result<usize> {
    println!("{}", color::section("AI Tools"));

    let mut tool_count = 0;

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
        tool_count += 1;
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
        } else {
            print!("  {}{:<24} configure? [Y/n] ", color::warn_mark(), name);
            if confirm_default_yes()? {
                configure(root, &member_id)?;
                configured = true;
            }
        }

        // Register as AI member only if tool was actually configured
        if configured && !project.members.contains_key(&member_id) {
            project.members.insert(
                member_id.clone(),
                joy_core::model::project::Member {
                    capabilities: joy_core::model::project::MemberCapabilities::Specific({
                        use joy_core::model::item::Capability;
                        use joy_core::model::project::CapabilityConfig;
                        let mut map = std::collections::BTreeMap::new();
                        // AI members get all work capabilities + create and assign,
                        // but NOT manage or delete
                        for cap in [
                            Capability::Conceive,
                            Capability::Plan,
                            Capability::Design,
                            Capability::Implement,
                            Capability::Test,
                            Capability::Review,
                            Capability::Document,
                            Capability::Create,
                            Capability::Assign,
                        ] {
                            map.insert(cap, CapabilityConfig::default());
                        }
                        map
                    }),
                },
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
        joy_core::store::write_yaml(&project_path, &project)?;
    }

    if tool_count == 0 {
        println!("  {}No supported AI tools detected.", color::warn_mark());
        println!(
            "  {}",
            color::inactive("Supported: Claude Code, Qwen Code, Mistral Vibe, GitHub Copilot")
        );
        println!(
            "  {}",
            color::inactive("Install one and re-run `joy ai setup`.")
        );
        println!();
        println!(
            "  {}",
            color::inactive("Templates in .joy/ai/ can be referenced manually from any AI tool.")
        );
    }

    println!();
    Ok(tool_count)
}

fn render_joy_block(member_id: &str, has_skill: bool) -> anyhow::Result<String> {
    let mut env = minijinja::Environment::new();
    env.add_template("block", JOY_BLOCK_TEMPLATE)?;
    let tmpl = env.get_template("block")?;
    let rendered = tmpl.render(minijinja::context! {
        member_id => member_id,
        has_skill => has_skill,
    })?;
    Ok(rendered.trim().to_string())
}

fn configure_claude(root: &Path, member_id: &str) -> anyhow::Result<bool> {
    let claude_dir = root.join(".claude");
    fs::create_dir_all(&claude_dir)?;
    let mut changed = false;

    let claude_md = claude_dir.join("CLAUDE.md");
    changed |= update_with_joy_block(&claude_md, &render_joy_block(member_id, true)?)?;
    qprintln!("    {}.claude/CLAUDE.md", color::check_mark());

    let skill_path = claude_dir.join("skills/joy/SKILL.md");
    changed |= write_if_changed(&skill_path, SKILL_TEMPLATE)?;
    qprintln!("    {}.claude/skills/joy/SKILL.md", color::check_mark());

    changed |= update_claude_permissions(root)?;

    Ok(changed)
}

fn update_claude_permissions(root: &Path) -> anyhow::Result<bool> {
    let settings_path = root.join(".claude/settings.json");
    let joy_permission = "Bash(joy *)";
    let jot_permission = "Bash(jot *)";
    let deprecated: &[(&str, &str)] = &[
        ("Bash(joy:*)", "Bash(joy *)"),
        ("Bash(jot:*)", "Bash(jot *)"),
    ];

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

    // Migrate deprecated colon syntax to space syntax
    for (old, new) in deprecated {
        if let Some(pos) = allow_arr.iter().position(|v| v.as_str() == Some(old)) {
            allow_arr[pos] = serde_json::json!(new);
        }
    }

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
    let mut changed = false;

    // Migrate: move root QWEN.md to .qwen/QWEN.md if it has a joy block
    let old_qwen_md = root.join("QWEN.md");
    let qwen_md = qwen_dir.join("QWEN.md");
    if old_qwen_md.is_file() && !qwen_md.is_file() {
        let content = fs::read_to_string(&old_qwen_md)?;
        if content.contains(JOY_BLOCK_START) {
            fs::rename(&old_qwen_md, &qwen_md)?;
            changed = true;
        }
    }

    changed |= update_with_joy_block(&qwen_md, &render_joy_block(member_id, true)?)?;
    qprintln!("    {}.qwen/QWEN.md", color::check_mark());

    let skill_path = qwen_dir.join("skills/joy/SKILL.md");
    changed |= write_if_changed(&skill_path, SKILL_TEMPLATE)?;
    qprintln!("    {}.qwen/skills/joy/SKILL.md", color::check_mark());

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

    let skill_path = vibe_dir.join("skills/joy/SKILL.md");
    let changed = write_if_changed(&skill_path, SKILL_TEMPLATE)?;
    qprintln!("    {}.vibe/skills/joy/SKILL.md", color::check_mark());
    qprintln!(
        "    {}",
        color::inactive("Note: set [tools.bash] permission = \"always\" in .vibe/config.toml")
    );

    Ok(changed)
}

fn configure_copilot(root: &Path, member_id: &str) -> anyhow::Result<bool> {
    let github_dir = root.join(".github");
    fs::create_dir_all(&github_dir)?;
    let mut changed = false;

    let instructions_md = github_dir.join("copilot-instructions.md");
    changed |= update_with_joy_block(&instructions_md, &render_joy_block(member_id, false)?)?;
    qprintln!("    {}.github/copilot-instructions.md", color::check_mark());

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
