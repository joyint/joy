// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::fs;
use std::io::Write;
use std::path::Path;

use joy_core::embedded::{self, EmbeddedFile, FileStatus};

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
}

pub fn run(args: AiArgs) -> anyhow::Result<()> {
    match args.command {
        AiCommand::Setup => setup(),
        AiCommand::Check => check(),
    }
}

fn setup() -> anyhow::Result<()> {
    let root = joy_core::store::find_project_root(&std::env::current_dir()?)
        .ok_or_else(|| anyhow::anyhow!("No Joy project found (run `joy init` first)"))?;

    println!("Setting up AI integration...\n");

    check_docs(&root)?;
    copy_templates(&root)?;
    configure_tools(&root)?;
    check_nested_projects(&root)?;

    println!("\nDone. AI tools can now use Joy in this project.");
    Ok(())
}

fn check_docs(root: &Path) -> anyhow::Result<()> {
    println!("Checking project documentation...");

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
            println!("  {} ... found", path);
        } else {
            println!("  {} ... MISSING", path);
            let name = path.rsplit('/').next().unwrap_or(path);
            print!(
                "  {} helps AI understand your {}. Create template? [Y/n] ",
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
                    "  Created {} (template -- your AI tool will help fill it in)",
                    path
                );
            }
            all_found = false;
        }
    }

    if all_found {
        println!("  All documentation present.");
    } else {
        println!("\n  Tip: Your AI tool will offer to fill in empty templates on first use.");
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
        println!("Up to date.");
        std::process::exit(0);
    }

    for (path, status) in &issues {
        let label = match status {
            FileStatus::Outdated => "outdated",
            FileStatus::Missing => "missing",
            FileStatus::UpToDate => unreachable!(),
        };
        println!("  {}: .joy/{}", label, path);
    }
    println!("\nRun `joy ai setup` to update.");
    std::process::exit(2);
}

fn copy_templates(root: &Path) -> anyhow::Result<()> {
    println!("Installing AI templates...");

    let actions = embedded::sync_files(root, AI_FILES)?;

    for action in &actions {
        println!("  .joy/{} ... {}", action.target, action.action);
    }

    println!();
    Ok(())
}

fn configure_tools(root: &Path) -> anyhow::Result<()> {
    println!("Detecting AI tools...");

    let mut found_any = false;

    if which("claude") {
        found_any = true;
        if is_tool_configured(root, "claude") {
            print!("  Claude Code (claude) ... configured.");
            configure_claude(root)?;
            println!();
        } else {
            print!("  Claude Code (claude) ... found. Configure? [Y/n] ");
            if confirm_default_yes()? {
                configure_claude(root)?;
            }
        }
    }
    if which("qwen") || which("qwen-code") {
        found_any = true;
        if is_tool_configured(root, "qwen") {
            print!("  Qwen Code (qwen) ... configured.");
            configure_qwen(root)?;
            println!();
        } else {
            print!("  Qwen Code (qwen) ... found. Configure? [Y/n] ");
            if confirm_default_yes()? {
                configure_qwen(root)?;
            }
        }
    }
    if which("vibe") {
        found_any = true;
        if is_tool_configured(root, "vibe") {
            print!("  Mistral Vibe (vibe) ... configured.");
            configure_vibe(root)?;
            println!();
        } else {
            print!("  Mistral Vibe (vibe) ... found. Configure? [Y/n] ");
            if confirm_default_yes()? {
                configure_vibe(root)?;
            }
        }
    }
    if which("copilot") || which("gh") {
        found_any = true;
        if is_tool_configured(root, "copilot") {
            print!("  GitHub Copilot (copilot) ... configured.");
            configure_copilot(root)?;
            println!();
        } else {
            print!("  GitHub Copilot (copilot) ... found. Configure? [Y/n] ");
            if confirm_default_yes()? {
                configure_copilot(root)?;
            }
        }
    }

    if !found_any {
        println!("  No supported AI tools detected.");
        println!("  Supported: Claude Code (claude), Qwen Code (qwen), Mistral Vibe (vibe), GitHub Copilot (copilot/gh)");
        println!("  Install one and re-run `joy ai setup`.");
        println!();
        println!("  The .joy/ai/ templates are installed regardless and can be");
        println!("  referenced manually from any AI tool's configuration.");
    }

    Ok(())
}

fn configure_claude(root: &Path) -> anyhow::Result<()> {
    let claude_dir = root.join(".claude");
    fs::create_dir_all(&claude_dir)?;

    // Update or create CLAUDE.md with joy block (preserves user content)
    let claude_md = claude_dir.join("CLAUDE.md");
    update_with_joy_block(
        &claude_md,
        "## Joy Integration\n\n\
         This project uses [Joy](https://github.com/joyint/joy) for product management.\n\
         Read [.joy/ai/instructions.md](../.joy/ai/instructions.md) for AI collaboration rules.\n\n\
         Use the `/joy` skill for backlog work. Do not edit `.joy/items/*.yaml` files directly.",
    )?;
    println!("    .claude/CLAUDE.md ... joy block updated");

    // Skill -- always update (Joy-owned)
    let skill_dir = claude_dir.join("skills/joy");
    fs::create_dir_all(&skill_dir)?;
    fs::write(skill_dir.join("SKILL.md"), SKILL_TEMPLATE)?;
    println!("    .claude/skills/joy/SKILL.md ... installed");

    // Permissions -- allow joy and jot commands without prompting
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

    // Ensure permissions.allow array exists and contains joy/jot
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
    println!("    .claude/settings.json ... joy/jot permissions added");

    Ok(())
}

fn configure_qwen(root: &Path) -> anyhow::Result<()> {
    let qwen_dir = root.join(".qwen");
    fs::create_dir_all(&qwen_dir)?;

    // Update or create QWEN.md with joy block (preserves user content)
    let qwen_md = root.join("QWEN.md");
    update_with_joy_block(
        &qwen_md,
        "## Joy Integration\n\n\
         This project uses [Joy](https://github.com/joyint/joy) for product management.\n\
         Read [.joy/ai/instructions.md](.joy/ai/instructions.md) for AI collaboration rules.\n\n\
         Use the `/joy` skill for backlog work. Do not edit `.joy/items/*.yaml` files directly.",
    )?;
    println!("    QWEN.md ... joy block updated");

    // Skill -- always update (Joy-owned)
    let skill_dir = qwen_dir.join("skills/joy");
    fs::create_dir_all(&skill_dir)?;
    fs::write(skill_dir.join("SKILL.md"), SKILL_TEMPLATE)?;
    println!("    .qwen/skills/joy/SKILL.md ... installed");

    // Permissions -- allow joy and jot commands without prompting
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
    println!("    .qwen/settings.json ... joy/jot permissions added");

    Ok(())
}

fn configure_vibe(root: &Path) -> anyhow::Result<()> {
    let vibe_dir = root.join(".vibe");
    fs::create_dir_all(&vibe_dir)?;

    // Skill -- always update (Joy-owned)
    let skill_dir = vibe_dir.join("skills/joy");
    fs::create_dir_all(&skill_dir)?;
    fs::write(skill_dir.join("SKILL.md"), SKILL_TEMPLATE)?;
    println!("    .vibe/skills/joy/SKILL.md ... installed");

    println!("    Note: Vibe does not support per-command permissions.");
    println!("    To auto-allow all shell commands: set [tools.bash] permission = \"always\" in .vibe/config.toml");

    Ok(())
}

fn configure_copilot(root: &Path) -> anyhow::Result<()> {
    let github_dir = root.join(".github");
    fs::create_dir_all(&github_dir)?;

    // Update or create copilot-instructions.md with joy block
    let instructions_md = github_dir.join("copilot-instructions.md");
    update_with_joy_block(
        &instructions_md,
        "## Joy Integration\n\n\
         This project uses [Joy](https://github.com/joyint/joy) for product management.\n\
         Read [.joy/ai/instructions.md](../.joy/ai/instructions.md) for AI collaboration rules.\n\n\
         Use Joy CLI commands for backlog work. Do not edit `.joy/items/*.yaml` files directly.",
    )?;
    println!("    .github/copilot-instructions.md ... joy block updated");

    println!(
        "    Note: Copilot does not support persistent per-command permissions in config files."
    );
    println!("    Use CLI flags to allow joy: gh copilot --allow-tool='shell(joy:*)'");

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
        println!("\nNested Joy projects without AI tool config:");
        for path in &unconfigured {
            println!("  {}/", path);
        }
        println!(
            "  AI tool permissions are per-project and not inherited from parent directories."
        );
        println!("  Run `joy ai setup` in each nested project to configure AI tools there.");
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
        "qwen" => root.join("QWEN.md").is_file(),
        "vibe" => root.join(".vibe/skills/joy/SKILL.md").is_file(),
        "copilot" => root.join(".github/copilot-instructions.md").is_file(),
        _ => false,
    }
}
