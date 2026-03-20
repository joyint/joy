// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::fs;
use std::io::Write;
use std::path::Path;

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
}

pub fn run(args: AiArgs) -> anyhow::Result<()> {
    match args.command {
        AiCommand::Setup => setup(),
    }
}

fn setup() -> anyhow::Result<()> {
    let root = joy_core::store::find_project_root(&std::env::current_dir()?)
        .ok_or_else(|| anyhow::anyhow!("No Joy project found (run `joy init` first)"))?;

    println!("Setting up AI integration...\n");

    check_docs(&root)?;
    copy_templates(&root)?;
    configure_tools(&root)?;

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

fn copy_templates(root: &Path) -> anyhow::Result<()> {
    println!("Installing AI templates...");

    let ai_dir = root.join(".joy/ai");
    fs::create_dir_all(&ai_dir)?;

    // instructions.md -- always update (Joy-owned)
    let instructions_path = ai_dir.join("instructions.md");
    fs::write(&instructions_path, INSTRUCTIONS_TEMPLATE)?;
    println!("  .joy/ai/instructions.md ... installed");

    // instructions/setup.md -- always update (Joy-owned)
    let setup_dir = ai_dir.join("instructions");
    fs::create_dir_all(&setup_dir)?;
    fs::write(setup_dir.join("setup.md"), SETUP_INSTRUCTIONS)?;
    println!("  .joy/ai/instructions/setup.md ... installed");

    // skills/joy/SKILL.md -- always update (Joy-owned)
    let skill_dir = ai_dir.join("skills/joy");
    fs::create_dir_all(&skill_dir)?;
    fs::write(skill_dir.join("SKILL.md"), SKILL_TEMPLATE)?;
    println!("  .joy/ai/skills/joy/SKILL.md ... installed");

    println!();
    Ok(())
}

fn configure_tools(root: &Path) -> anyhow::Result<()> {
    println!("Detecting AI tools...");

    let mut found_any = false;

    if which("claude") {
        print!("  Claude Code (claude) ... found. Configure? [Y/n] ");
        if confirm_default_yes()? {
            configure_claude(root)?;
        }
        found_any = true;
    }
    if which("qwen") || which("qwen-code") {
        print!("  Qwen Code (qwen) ... found. Configure? [Y/n] ");
        if confirm_default_yes()? {
            configure_qwen(root)?;
        }
        found_any = true;
    }
    if which("vibe") {
        print!("  Mistral Vibe (vibe) ... found. Configure? [Y/n] ");
        if confirm_default_yes()? {
            configure_vibe(root)?;
        }
        found_any = true;
    }
    if which("copilot") || which("gh") {
        print!("  GitHub Copilot (copilot) ... found. Configure? [Y/n] ");
        if confirm_default_yes()? {
            configure_copilot(root)?;
        }
        found_any = true;
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
