// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::fs;
use std::io::Write;
use std::path::Path;

const INSTRUCTIONS_TEMPLATE: &str = include_str!("../../../../data/ai/instructions.md");
const SKILL_TEMPLATE: &str = include_str!("../../../../data/ai/skills/joy/SKILL.md");

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

    // Phase 1: Check documentation
    check_docs(&root)?;

    // Phase 2: Copy AI templates
    copy_templates(&root)?;

    // Phase 3: Detect and configure AI tools
    configure_tools(&root)?;

    println!("\nDone. AI tools can now use Joy in this project.");
    Ok(())
}

fn check_docs(root: &Path) -> anyhow::Result<()> {
    println!("Checking project documentation...");

    let docs = [
        ("docs/dev/Vision.md", "product goals and design decisions"),
        ("docs/dev/Architecture.md", "technical stack and structure"),
        ("CONTRIBUTING.md", "coding conventions and commit messages"),
    ];

    let mut all_found = true;
    for (path, purpose) in &docs {
        let full = root.join(path);
        if full.is_file() {
            println!("  {} ... found", path);
        } else {
            println!("  {} ... MISSING", path);
            println!(
                "  A {} document helps AI tools understand your {}.",
                path.rsplit('/').next().unwrap_or(path),
                purpose
            );
            all_found = false;
        }
    }

    if all_found {
        println!("  All documentation present.\n");
    } else {
        println!(
            "\n  Tip: Create missing documents to improve AI collaboration quality.\n  The AI will review them on first use and may suggest improvements.\n"
        );
    }

    Ok(())
}

fn copy_templates(root: &Path) -> anyhow::Result<()> {
    println!("Installing AI templates...");

    let ai_dir = root.join(".joy/ai");
    fs::create_dir_all(&ai_dir)?;

    let instructions_path = ai_dir.join("instructions.md");
    fs::write(&instructions_path, INSTRUCTIONS_TEMPLATE)?;
    println!("  .joy/ai/instructions.md ... installed");

    let skill_dir = ai_dir.join("skills/joy");
    fs::create_dir_all(&skill_dir)?;
    let skill_path = skill_dir.join("SKILL.md");
    fs::write(&skill_path, SKILL_TEMPLATE)?;
    println!("  .joy/ai/skills/joy/SKILL.md ... installed");

    println!();
    Ok(())
}

fn configure_tools(root: &Path) -> anyhow::Result<()> {
    println!("Detecting AI tools...");

    let mut found_any = false;

    if which("claude") {
        println!("  Claude Code (claude) ... found");
        configure_claude(root)?;
        found_any = true;
    }
    if which("qwen-code") {
        println!("  Qwen Code (qwen-code) ... found");
        configure_qwen(root)?;
        found_any = true;
    }
    if which("vibe") {
        println!("  Mistral Vibe (vibe) ... found");
        configure_vibe(root)?;
        found_any = true;
    }

    if !found_any {
        println!("  No supported AI tools detected.");
        println!("  Supported: Claude Code (claude), Qwen Code (qwen-code), Mistral Vibe (vibe)");
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

    // Update or create CLAUDE.md with joy block
    let claude_md = claude_dir.join("CLAUDE.md");
    update_with_joy_block(
        &claude_md,
        "## Joy Integration\n\n\
         This project uses [Joy](https://github.com/joyint/joy) for product management.\n\
         Read [.joy/ai/instructions.md](../.joy/ai/instructions.md) for AI collaboration rules.\n\n\
         Use the `/joy` skill for backlog work. Do not edit `.joy/items/*.yaml` files directly.",
    )?;
    println!("    .claude/CLAUDE.md ... updated");

    // Copy skill
    let skill_dir = claude_dir.join("skills/joy");
    fs::create_dir_all(&skill_dir)?;
    fs::write(skill_dir.join("SKILL.md"), SKILL_TEMPLATE)?;
    println!("    .claude/skills/joy/SKILL.md ... installed");

    Ok(())
}

fn configure_qwen(root: &Path) -> anyhow::Result<()> {
    let qwen_dir = root.join(".qwen");
    fs::create_dir_all(&qwen_dir)?;

    // Update or create QWEN.md with joy block
    let qwen_md = root.join("QWEN.md");
    update_with_joy_block(
        &qwen_md,
        "## Joy Integration\n\n\
         This project uses [Joy](https://github.com/joyint/joy) for product management.\n\
         Read [.joy/ai/instructions.md](.joy/ai/instructions.md) for AI collaboration rules.\n\n\
         Use the `/joy` skill for backlog work. Do not edit `.joy/items/*.yaml` files directly.",
    )?;
    println!("    QWEN.md ... updated");

    // Copy skill
    let skill_dir = qwen_dir.join("skills/joy");
    fs::create_dir_all(&skill_dir)?;
    fs::write(skill_dir.join("SKILL.md"), SKILL_TEMPLATE)?;
    println!("    .qwen/skills/joy/SKILL.md ... installed");

    Ok(())
}

fn configure_vibe(root: &Path) -> anyhow::Result<()> {
    let vibe_dir = root.join(".vibe");
    fs::create_dir_all(&vibe_dir)?;

    // Copy skill to vibe skills directory
    let skill_dir = vibe_dir.join("skills/joy");
    fs::create_dir_all(&skill_dir)?;
    fs::write(skill_dir.join("SKILL.md"), SKILL_TEMPLATE)?;
    println!("    .vibe/skills/joy/SKILL.md ... installed");

    Ok(())
}

fn update_with_joy_block(path: &Path, content: &str) -> anyhow::Result<()> {
    let block = format!("{}\n{}\n{}", JOY_BLOCK_START, content, JOY_BLOCK_END);

    if path.is_file() {
        let existing = fs::read_to_string(path)?;
        if existing.contains(JOY_BLOCK_START) && existing.contains(JOY_BLOCK_END) {
            // Replace existing block
            let start = existing.find(JOY_BLOCK_START).unwrap();
            let end = existing.find(JOY_BLOCK_END).unwrap() + JOY_BLOCK_END.len();
            let mut updated = String::new();
            updated.push_str(&existing[..start]);
            updated.push_str(&block);
            updated.push_str(&existing[end..]);
            fs::write(path, updated)?;
        } else {
            // Append block
            let mut file = fs::OpenOptions::new().append(true).open(path)?;
            writeln!(file, "\n{}", block)?;
        }
    } else {
        // Create new file
        fs::write(path, format!("{}\n", block))?;
    }

    Ok(())
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
