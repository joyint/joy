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

    let msg = format!("AI integration complete -- {} tool(s) configured", tool_count);
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
        ("GitHub Copilot", "copilot", &[".github/copilot-instructions.md"]),
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
        println!("{}No AI tool configurations found.", color::check_mark());
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

    let count = tools.iter().filter(|(_, _, paths)| {
        paths.iter().any(|p| to_remove.iter().any(|(_, tp)| tp == p))
    }).count();
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
        println!("  {}{:<32} {}", color::check_mark(), format!(".joy/{}", action.target), status);
    }

    println!();
    Ok(())
}

fn configure_tools(root: &Path) -> anyhow::Result<usize> {
    println!("{}", color::section("AI Tools"));

    let mut tool_count = 0;

    let tools: &[(&str, &str, fn(bool) -> bool, fn(&Path) -> anyhow::Result<()>)] = &[
        ("Claude Code", "claude", |_| which("claude"), configure_claude),
        ("Qwen Code", "qwen", |_| which("qwen") || which("qwen-code"), configure_qwen),
        ("Mistral Vibe", "vibe", |_| which("vibe"), configure_vibe),
        ("GitHub Copilot", "copilot", |_| which("copilot") || which("gh"), configure_copilot),
    ];

    for (name, id, detect, configure) in tools {
        if !detect(false) {
            continue;
        }
        tool_count += 1;
        let already = is_tool_configured(root, id);
        if already {
            // Silently update files, then show single status line
            QUIET.store(true, std::sync::atomic::Ordering::Relaxed);
            configure(root)?;
            QUIET.store(false, std::sync::atomic::Ordering::Relaxed);
            println!("  {}{:<24} {}", color::check_mark(), name, color::inactive("up to date"));
        } else {
            print!("  {}{:<24} configure? [Y/n] ", color::warn_mark(), name);
            if confirm_default_yes()? {
                configure(root)?;
            }
        }
    }

    if tool_count == 0 {
        println!("  {}No supported AI tools detected.", color::warn_mark());
        println!("  {}", color::inactive("Supported: Claude Code, Qwen Code, Mistral Vibe, GitHub Copilot"));
        println!("  {}", color::inactive("Install one and re-run `joy ai setup`."));
        println!();
        println!("  {}", color::inactive("Templates in .joy/ai/ can be referenced manually from any AI tool."));
    }

    println!();
    Ok(tool_count)
}

fn configure_claude(root: &Path) -> anyhow::Result<()> {
    let claude_dir = root.join(".claude");
    fs::create_dir_all(&claude_dir)?;

    let claude_md = claude_dir.join("CLAUDE.md");
    update_with_joy_block(
        &claude_md,
        "## Joy Integration\n\n\
         This project uses [Joy](https://github.com/joyint/joy) for product management.\n\
         Read [.joy/ai/instructions.md](../.joy/ai/instructions.md) for AI collaboration rules.\n\n\
         Use the `/joy` skill for backlog work. Do not edit `.joy/items/*.yaml` files directly.",
    )?;
    qprintln!("    {}.claude/CLAUDE.md", color::check_mark());

    let skill_dir = claude_dir.join("skills/joy");
    fs::create_dir_all(&skill_dir)?;
    fs::write(skill_dir.join("SKILL.md"), SKILL_TEMPLATE)?;
    qprintln!("    {}.claude/skills/joy/SKILL.md", color::check_mark());

    update_claude_permissions(root)?;

    Ok(())
}

fn update_claude_permissions(root: &Path) -> anyhow::Result<()> {
    let settings_path = root.join(".claude/settings.json");
    let joy_permission = "Bash(joy:*)";
    let jot_permission = "Bash(jot:*)";

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
    fs::write(&settings_path, format!("{json}\n"))?;
    qprintln!("    {}.claude/settings.json", color::check_mark());

    Ok(())
}

fn configure_qwen(root: &Path) -> anyhow::Result<()> {
    let qwen_dir = root.join(".qwen");
    fs::create_dir_all(&qwen_dir)?;

    // Migrate: move root QWEN.md to .qwen/QWEN.md if it has a joy block
    let old_qwen_md = root.join("QWEN.md");
    let qwen_md = qwen_dir.join("QWEN.md");
    if old_qwen_md.is_file() && !qwen_md.is_file() {
        let content = fs::read_to_string(&old_qwen_md)?;
        if content.contains(JOY_BLOCK_START) {
            fs::rename(&old_qwen_md, &qwen_md)?;
        }
    }

    update_with_joy_block(
        &qwen_md,
        "## Joy Integration\n\n\
         This project uses [Joy](https://github.com/joyint/joy) for product management.\n\
         Read [.joy/ai/instructions.md](../.joy/ai/instructions.md) for AI collaboration rules.\n\n\
         Use the `/joy` skill for backlog work. Do not edit `.joy/items/*.yaml` files directly.",
    )?;
    qprintln!("    {}.qwen/QWEN.md", color::check_mark());

    let skill_dir = qwen_dir.join("skills/joy");
    fs::create_dir_all(&skill_dir)?;
    fs::write(skill_dir.join("SKILL.md"), SKILL_TEMPLATE)?;
    qprintln!("    {}.qwen/skills/joy/SKILL.md", color::check_mark());

    update_qwen_permissions(root)?;

    Ok(())
}

fn update_qwen_permissions(root: &Path) -> anyhow::Result<()> {
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
    fs::write(&settings_path, format!("{json}\n"))?;
    qprintln!("    {}.qwen/settings.json", color::check_mark());

    Ok(())
}

fn configure_vibe(root: &Path) -> anyhow::Result<()> {
    let vibe_dir = root.join(".vibe");
    fs::create_dir_all(&vibe_dir)?;

    let skill_dir = vibe_dir.join("skills/joy");
    fs::create_dir_all(&skill_dir)?;
    fs::write(skill_dir.join("SKILL.md"), SKILL_TEMPLATE)?;
    qprintln!("    {}.vibe/skills/joy/SKILL.md", color::check_mark());
    qprintln!("    {}", color::inactive("Note: set [tools.bash] permission = \"always\" in .vibe/config.toml"));

    Ok(())
}

fn configure_copilot(root: &Path) -> anyhow::Result<()> {
    let github_dir = root.join(".github");
    fs::create_dir_all(&github_dir)?;

    let instructions_md = github_dir.join("copilot-instructions.md");
    update_with_joy_block(
        &instructions_md,
        "## Joy Integration\n\n\
         This project uses [Joy](https://github.com/joyint/joy) for product management.\n\
         Read [.joy/ai/instructions.md](../.joy/ai/instructions.md) for AI collaboration rules.\n\n\
         Use Joy CLI commands for backlog work. Do not edit `.joy/items/*.yaml` files directly.",
    )?;
    qprintln!("    {}.github/copilot-instructions.md", color::check_mark());
    qprintln!("    {}", color::inactive("Note: use gh copilot --allow-tool='shell(joy:*)' for permissions"));

    Ok(())
}

fn update_with_joy_block(path: &Path, content: &str) -> anyhow::Result<()> {
    let block = format!("{}\n{}\n{}", JOY_BLOCK_START, content, JOY_BLOCK_END);

    if path.is_file() {
        let existing = fs::read_to_string(path)?;
        if existing.contains(JOY_BLOCK_START) && existing.contains(JOY_BLOCK_END) {
            // Replace existing joy block, preserve everything else
            let start = existing.find(JOY_BLOCK_START).unwrap();
            let end = existing.find(JOY_BLOCK_END).unwrap() + JOY_BLOCK_END.len();
            let mut updated = String::new();
            updated.push_str(&existing[..start]);
            updated.push_str(&block);
            updated.push_str(&existing[end..]);
            fs::write(path, updated)?;
        } else {
            // Append joy block to existing file
            let mut file = fs::OpenOptions::new().append(true).open(path)?;
            writeln!(file, "\n{}", block)?;
        }
    } else {
        // Create new file with joy block
        fs::write(path, format!("{}\n", block))?;
    }

    Ok(())
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
        println!("  {}", color::inactive("Permissions are per-project. Run `joy ai setup` in each."));
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
