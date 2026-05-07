// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use joy_core::ai_templates;
use joy_core::vcs::Vcs;

use crate::color;

static QUIET: AtomicBool = AtomicBool::new(false);

macro_rules! qprintln {
    ($($arg:tt)*) => {
        if !QUIET.load(std::sync::atomic::Ordering::Relaxed)
            && !crate::output::is_json()
        {
            println!($($arg)*);
        }
    };
}

/// Display-only println: suppressed in JSON mode.
macro_rules! dprintln {
    ($($arg:tt)*) => {
        if !crate::output::is_json() {
            println!($($arg)*);
        }
    };
}

macro_rules! dprint {
    ($($arg:tt)*) => {
        if !crate::output::is_json() {
            print!($($arg)*);
        }
    };
}

const VISION_TEMPLATE: &str = include_str!("../../docs/vision/README.md");
const ARCHITECTURE_TEMPLATE: &str = include_str!("../../docs/architecture/README.md");
const CONTRIBUTING_TEMPLATE: &str = include_str!("../../docs/CONTRIBUTING.md");

const JOY_BLOCK_START: &str = "<!-- joy:start -->";
const JOY_BLOCK_END: &str = "<!-- joy:end -->";

#[derive(clap::Args)]
pub struct AiArgs {
    #[command(subcommand)]
    command: AiCommand,
}

#[derive(clap::Subcommand)]
enum AiCommand {
    /// Initialize AI tool integration for new tools
    Init(InitArgs),
    /// Remove AI tool configurations from this project
    Reset(ResetArgs),
    /// Rotate the (operator, AI) delegation keypair
    Rotate(RotateArgs),
}

#[derive(clap::Args, Default)]
struct InitArgs {
    /// Path to the architecture doc (e.g. docs/architecture/README.md). Skips the prompt.
    #[arg(long, value_hint = clap::ValueHint::FilePath)]
    architecture: Option<String>,

    /// Path to the vision doc (e.g. docs/vision/README.md). Skips the prompt.
    #[arg(long, value_hint = clap::ValueHint::FilePath)]
    vision: Option<String>,

    /// Path to the contributing doc (e.g. CONTRIBUTING.md). Skips the prompt.
    #[arg(long, value_hint = clap::ValueHint::FilePath)]
    contributing: Option<String>,

    /// Passphrase (non-interactive, for scripts and tests). Required for
    /// signing the attestation that ties newly registered AI members to
    /// the acting manage member.
    #[arg(long)]
    passphrase: Option<String>,
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

#[derive(clap::Args)]
struct RotateArgs {
    /// AI member ID whose delegation to rotate (e.g. ai:claude@joy).
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_ai_member))]
    member: String,

    /// Passphrase (non-interactive, for scripts and tests).
    #[arg(long)]
    passphrase: Option<String>,
}

pub fn run(args: AiArgs) -> anyhow::Result<()> {
    match args.command {
        AiCommand::Init(a) => ai_init(a),
        AiCommand::Reset(a) => reset(a),
        AiCommand::Rotate(a) => {
            crate::commands::auth::run_ai_rotate(&a.member, a.passphrase.as_deref())
        }
    }
}

/// Run the AI init flow with default prompts. Used by the `joy` welcome
/// wizard after a fresh `joy init`.
pub fn run_init_default() -> anyhow::Result<()> {
    ai_init(InitArgs::default())
}

fn ai_init(args: InitArgs) -> anyhow::Result<()> {
    let root = joy_core::store::find_project_root(&std::env::current_dir()?)
        .ok_or_else(|| anyhow::anyhow!("No Joy project found (run `joy init` first)"))?;

    dprintln!("{}", color::header("AI Init"));
    dprintln!();

    // Ensure project.defaults.yaml exists
    joy_core::embedded::sync_files(&root, joy_core::init::PROJECT_FILES)?;

    check_docs(&root, &args)?;
    let configured_tools = setup_new_tools(&root, args.passphrase.as_deref())?;
    update_gitignore(&root, &configured_tools)?;
    check_nested_projects(&root)?;

    if crate::output::is_json() {
        return crate::output::emit(AiInitPayload {
            configured_tools: configured_tools.iter().map(|s| s.to_string()).collect(),
        });
    }

    let msg = format!(
        "AI integration complete -- {}",
        color::plural(configured_tools.len(), "tool")
    );
    dprintln!("{}", color::footer(&msg));
    Ok(())
}

#[derive(serde::Serialize)]
struct AiInitPayload {
    configured_tools: Vec<String>,
}

/// Programmatic entry to the AI update routine, invoked from the
/// `joy update` orchestrator and the auto-sync hook so configured AI
/// tool files refresh after a binary upgrade.
pub(crate) fn run_update_default() -> anyhow::Result<()> {
    update(false).map(|_| ())
}

/// Read-only check used by `joy update --check`. Prints one line per
/// configured AI tool and returns `true` when anything is stale.
pub(crate) fn run_check_default() -> anyhow::Result<bool> {
    update(true)
}

fn update(dry_run: bool) -> anyhow::Result<bool> {
    let root = joy_core::store::find_project_root(&std::env::current_dir()?)
        .ok_or_else(|| anyhow::anyhow!("No Joy project found (run `joy init` first)"))?;

    if crate::output::is_json() {
        let mut entries: Vec<AiToolEntry> = Vec::new();
        let mut has_issues = false;
        for (name, id, detect, _configure) in ALL_TOOLS {
            let installed = detect();
            let configured = is_tool_configured(&root, id);
            let stale = if dry_run && configured {
                let member_id = format!("ai:{id}@joy");
                is_tool_stale(&root, id, &member_id)?
            } else {
                false
            };
            if stale {
                has_issues = true;
            }
            entries.push(AiToolEntry {
                name: name.to_string(),
                id: id.to_string(),
                installed,
                configured,
                stale,
            });
        }
        crate::output::emit(AiStatusPayload {
            check: dry_run,
            tools: entries,
            has_issues,
        })?;
        return Ok(has_issues);
    }

    if !dry_run {
        // Full-width header / sync only on the write path. The dry-run
        // path is invoked from `joy update --check`, which provides
        // the surrounding header and section title itself.
        dprintln!("{}", color::header("AI Update"));
        dprintln!();
        joy_core::embedded::sync_files(&root, joy_core::init::PROJECT_FILES)?;
        dprintln!("{}", color::section("AI Tools"));
    }

    let mut tool_count = 0;
    let mut has_issues = false;
    let mut configured_tools: Vec<&str> = Vec::new();

    for (name, id, detect, configure) in ALL_TOOLS {
        let installed = detect();
        let configured = is_tool_configured(&root, id);
        if configured {
            tool_count += 1;
            configured_tools.push(id);
            let member_id = format!("ai:{id}@joy");
            if dry_run {
                let stale = is_tool_stale(&root, id, &member_id)?;
                if stale {
                    has_issues = true;
                    dprintln!(
                        "  {}{:<24} {}",
                        color::warn_mark(),
                        name,
                        color::warning("outdated")
                    );
                } else {
                    dprintln!(
                        "  {}{:<24} {}",
                        color::check_mark(),
                        name,
                        color::inactive("up to date")
                    );
                }
            } else {
                QUIET.store(true, std::sync::atomic::Ordering::Relaxed);
                let changed = configure(&root, &member_id)?;
                QUIET.store(false, std::sync::atomic::Ordering::Relaxed);
                let status = if changed {
                    color::success("updated")
                } else {
                    color::inactive("up to date")
                };
                dprintln!("  {}{:<24} {}", color::check_mark(), name, status);
            }
        } else if installed {
            dprintln!(
                "  {}{:<24} {}",
                color::warn_mark(),
                name,
                color::warning("installed, not configured")
            );
        } else {
            dprintln!(
                "  {}{:<24} {}",
                color::empty_mark(),
                name,
                color::inactive("not installed")
            );
        }
    }

    if tool_count == 0 && !dry_run {
        dprintln!(
            "  {}No configured AI tools found. Run {} first.",
            color::warn_mark(),
            color::label("joy ai init")
        );
    }

    // Sync gitignore entries for configured tools
    if !dry_run {
        update_gitignore(&root, &configured_tools)?;
    }

    if !dry_run {
        // The aggregated joy update --check provides its own footer;
        // the per-section dry-run path stays footer-less.
        dprintln!();
        let msg = format!(
            "AI update complete -- {}",
            color::plural(tool_count, "tool")
        );
        dprintln!("{}", color::footer(&msg));
    }

    Ok(has_issues)
}

/// Check if a tool's generated files are up to date by re-rendering expected
/// content and comparing against on-disk files.
fn is_tool_stale(root: &Path, tool: &str, member_id: &str) -> anyhow::Result<bool> {
    let workflow = ai_templates::load_workflow()?;
    let agents = ai_templates::load_agents()?;

    // Check SKILL.md (all tools except copilot)
    let skill_path = match tool {
        "claude" => Some(root.join(".claude/skills/joy/SKILL.md")),
        "qwen" => Some(root.join(".qwen/skills/joy/SKILL.md")),
        "vibe" => Some(root.join(".vibe/skills/joy/SKILL.md")),
        _ => None,
    };
    if let Some(path) = skill_path {
        let expected = ai_templates::render_skill(&workflow)?;
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
        if !file_matches(&path, ai_templates::setup_instructions()) {
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
        let expected_block = render_managed_block(member_id, has_skill)?;
        if !joy_block_matches(&path, &expected_block) {
            return Ok(true);
        }
    }

    // Check copilot prompt
    if tool == "copilot" {
        let expected = ai_templates::render_copilot_prompt(&workflow)?;
        if !file_matches(&root.join(".github/prompts/joy.prompt.md"), &expected) {
            return Ok(true);
        }
    }

    // Check agent files
    for agent in &agents {
        if !ai_templates::agent_applicable_to_tool(agent, tool) {
            continue;
        }
        if let Some(filename) = ai_templates::agent_filename(agent, tool) {
            let expected = ai_templates::render_agent(agent, &workflow, tool)?;
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

/// Check if a file's content matches the expected content exactly.
fn file_matches(path: &Path, expected: &str) -> bool {
    match fs::read_to_string(path) {
        Ok(content) => content == expected,
        Err(_) => false,
    }
}

/// Check if the joy-block inside a file matches the expected block content.
fn joy_block_matches(path: &Path, expected_block: &str) -> bool {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let expected_wrapped = format!("{}\n{}\n{}", JOY_BLOCK_START, expected_block, JOY_BLOCK_END);
    content.contains(&expected_wrapped)
}

/// One configurable doc the project tracks for AI tools.
struct DocSpec {
    /// Logical key under `project.docs.*` (architecture / vision / contributing).
    key: &'static str,
    /// Human label shown in the prompt.
    label: &'static str,
    /// Why this doc helps the AI (shown when offering a template stub).
    purpose: &'static str,
    /// Built-in default path used when nothing is configured / scanned.
    default_path: &'static str,
    /// Candidate paths to scan in priority order. Used to suggest existing docs
    /// in repos that already follow a different convention.
    candidates: &'static [&'static str],
    /// Embedded template content to seed a missing file.
    template: &'static str,
}

const DOC_SPECS: &[DocSpec] = &[
    DocSpec {
        key: "vision",
        label: "Vision",
        purpose: "product goals and design decisions",
        default_path: joy_core::model::project::Docs::DEFAULT_VISION,
        candidates: &[
            "docs/dev/vision/README.md",
            "docs/vision/README.md",
            "docs/vision.md",
            "VISION.md",
        ],
        template: VISION_TEMPLATE,
    },
    DocSpec {
        key: "architecture",
        label: "Architecture",
        purpose: "technical stack and structure",
        default_path: joy_core::model::project::Docs::DEFAULT_ARCHITECTURE,
        candidates: &[
            "docs/dev/architecture/README.md",
            "docs/architecture/README.md",
            "docs/architecture.md",
            "ARCHITECTURE.md",
        ],
        template: ARCHITECTURE_TEMPLATE,
    },
    DocSpec {
        key: "contributing",
        label: "Contributing",
        purpose: "coding conventions and commit messages",
        default_path: joy_core::model::project::Docs::DEFAULT_CONTRIBUTING,
        candidates: &[
            "CONTRIBUTING.md",
            "docs/CONTRIBUTING.md",
            ".github/CONTRIBUTING.md",
        ],
        template: CONTRIBUTING_TEMPLATE,
    },
];

/// How a resolved doc path was obtained. Drives the output annotation and the
/// "tip" hint at the end of `check_docs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocPathSource {
    /// Provided via `--vision` / `--architecture` / `--contributing` flag.
    Flag,
    /// Already stored in `project.yaml` under `docs.*`.
    Configured,
    /// Discovered on disk by scanning `spec.candidates`.
    AutoDetected,
    /// User answered an interactive prompt because nothing was detected.
    Prompted,
}

struct ResolvedDoc {
    path: String,
    source: DocPathSource,
}

fn check_docs(root: &Path, args: &InitArgs) -> anyhow::Result<()> {
    use joy_core::model::Project;

    dprintln!("{}", color::section("Documentation"));

    let project_path = joy_core::store::joy_dir(root).join(joy_core::store::PROJECT_FILE);
    let mut project: Project = joy_core::store::read_yaml(&project_path)?;
    let mut project_changed = false;
    let mut all_found = true;
    let mut any_auto_detected = false;

    for spec in DOC_SPECS {
        let configured = match spec.key {
            "vision" => project.docs.vision.clone(),
            "architecture" => project.docs.architecture.clone(),
            "contributing" => project.docs.contributing.clone(),
            _ => unreachable!(),
        };
        let flag_override = match spec.key {
            "vision" => args.vision.clone(),
            "architecture" => args.architecture.clone(),
            "contributing" => args.contributing.clone(),
            _ => unreachable!(),
        };

        let mut resolved =
            resolve_doc_path(root, spec, configured.as_deref(), flag_override.as_deref())?;

        // If the configured path no longer points to a file but a candidate
        // location does, offer to switch instead of creating a duplicate
        // template at the stale path.
        if resolved.source == DocPathSource::Configured && !root.join(&resolved.path).is_file() {
            let suggestion = suggested_doc_path(root, spec);
            if suggestion != resolved.path && root.join(suggestion).is_file() {
                dprintln!(
                    "  {}{}",
                    color::warn_mark(),
                    color::warning(&format!(
                        "configured {} not found, but {} exists",
                        resolved.path, suggestion
                    ))
                );
                dprint!("    Switch to {}? [Y/n] ", suggestion);
                std::io::stdout().flush()?;
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                let trimmed = input.trim();
                if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("y") {
                    resolved = ResolvedDoc {
                        path: suggestion.to_string(),
                        source: DocPathSource::AutoDetected,
                    };
                }
            }
        }

        let chosen = resolved.path.clone();

        // Persist non-default choices so AI tools and future runs see them.
        let to_store = if chosen == spec.default_path {
            None
        } else {
            Some(chosen.clone())
        };
        if to_store != configured {
            match spec.key {
                "vision" => project.docs.vision = to_store,
                "architecture" => project.docs.architecture = to_store,
                "contributing" => project.docs.contributing = to_store,
                _ => unreachable!(),
            }
            project_changed = true;
        }

        if resolved.source == DocPathSource::AutoDetected {
            any_auto_detected = true;
        }

        let full = root.join(&chosen);
        if full.is_file() {
            if resolved.source == DocPathSource::AutoDetected {
                dprintln!(
                    "  {}{} {}",
                    color::check_mark(),
                    chosen,
                    color::inactive("(auto-detected)")
                );
            } else {
                dprintln!("  {}{}", color::check_mark(), chosen);
            }
        } else {
            dprintln!("  {}{}", color::cross_mark(), color::warning(&chosen));
            let name = chosen.rsplit('/').next().unwrap_or(&chosen);
            dprint!(
                "    {} helps AI understand your {}. Create template? [Y/n] ",
                name,
                spec.purpose
            );
            std::io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let trimmed = input.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("y") {
                if let Some(parent) = full.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&full, spec.template)?;
                dprintln!(
                    "    {}Created {} (template -- your AI tool will help fill it in)",
                    color::check_mark(),
                    chosen
                );
            }
            all_found = false;
        }
    }

    if project_changed {
        joy_core::store::write_yaml_preserve(&project_path, &project)?;
    }

    if any_auto_detected {
        dprintln!(
            "\n  {}",
            color::inactive("Tip: change doc paths with `joy project set docs.<key> <path>`")
        );
    }

    if !all_found {
        dprintln!(
            "\n  {}Your AI tool will offer to fill in empty templates on first use.",
            color::warn_mark()
        );
    }
    dprintln!();

    Ok(())
}

/// Pick a suggestion for a doc path: the first candidate that exists, or the
/// built-in default if none match. Does no other IO.
fn suggested_doc_path<'a>(root: &Path, spec: &'a DocSpec) -> &'a str {
    spec.candidates
        .iter()
        .find(|p| root.join(p).is_file())
        .copied()
        .unwrap_or(spec.default_path)
}

/// Decide which path to use for a given doc spec.
///
/// Precedence:
/// 1. Flag value passed to `joy ai init` (non-interactive).
/// 2. Path already stored in `project.docs` (no prompt).
/// 3. Auto-detection: first existing candidate file (no prompt).
/// 4. Interactive prompt, defaulting to the built-in default path.
fn resolve_doc_path(
    root: &Path,
    spec: &DocSpec,
    configured: Option<&str>,
    flag: Option<&str>,
) -> anyhow::Result<ResolvedDoc> {
    if let Some(flag_value) = flag {
        let trimmed = flag_value.trim();
        if !trimmed.is_empty() {
            return Ok(ResolvedDoc {
                path: trimmed.to_string(),
                source: DocPathSource::Flag,
            });
        }
    }
    if let Some(value) = configured {
        return Ok(ResolvedDoc {
            path: value.to_string(),
            source: DocPathSource::Configured,
        });
    }

    let suggestion = suggested_doc_path(root, spec);
    if root.join(suggestion).is_file() {
        return Ok(ResolvedDoc {
            path: suggestion.to_string(),
            source: DocPathSource::AutoDetected,
        });
    }

    dprint!("    {} doc path [{}]: ", spec.label, suggestion);
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    let path = if trimmed.is_empty() {
        suggestion.to_string()
    } else {
        trimmed.to_string()
    };
    Ok(ResolvedDoc {
        path,
        source: DocPathSource::Prompted,
    })
}

fn reset(args: ResetArgs) -> anyhow::Result<()> {
    let root = joy_core::store::find_project_root(&std::env::current_dir()?)
        .ok_or_else(|| anyhow::anyhow!("No Joy project found (run `joy init` first)"))?;

    // Per tool: joy-managed paths only. Shared files (CLAUDE.md, QWEN.md,
    // AGENTS.md, copilot-instructions.md) are listed separately so reset
    // strips only the joy block instead of deleting the whole file
    // (JOY-00D1-3C).
    let all_tools: &[(&str, &str, &[&str])] = &[
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
        if let Ok(mut project) = joy_core::store::read_project(&project_path) {
            let mut cleaned = false;
            for (_, id, _) in &tools {
                let member_id = format!("ai:{id}@joy");
                if project.members.remove(&member_id).is_some() {
                    dprintln!(
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
                dprintln!("{}No AI tool configurations found.", color::check_mark());
            }
        } else {
            dprintln!("{}No AI tool configurations found.", color::check_mark());
        }
        return Ok(());
    }

    dprintln!("{}", color::header("AI Reset"));
    dprintln!();
    dprintln!("Will remove:");
    for (name, path) in &to_remove {
        dprintln!("  {}{:<24} {}", color::cross_mark(), name, path);
    }

    if !args.force {
        dprintln!();
        dprint!("Proceed? [y/N] ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let trimmed = input.trim();
        if !trimmed.eq_ignore_ascii_case("y") {
            dprintln!("Aborted.");
            return Ok(());
        }
    }

    for (name, path) in &to_remove {
        let full = root.join(path);
        // Shared instruction files: strip the joy-block but keep the rest
        // intact. Joy-only paths (skills/joy/, agents/, prompts/, ...)
        // are safe to remove wholesale.
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
        dprintln!("  {}{:<24} removed", color::check_mark(), name);
    }

    // Remove AI members from project.yaml for reset tools
    let project_path = joy_core::store::joy_dir(&root).join(joy_core::store::PROJECT_FILE);
    if let Ok(mut project) = joy_core::store::read_project(&project_path) {
        let mut project_changed = false;
        for (_, id, paths) in &tools {
            let was_removed = paths
                .iter()
                .any(|p| to_remove.iter().any(|(_, tp)| tp == p));
            if was_removed {
                let member_id = format!("ai:{id}@joy");
                if project.members.remove(&member_id).is_some() {
                    dprintln!("  {}{:<24} member removed", color::check_mark(), member_id);
                    project_changed = true;
                    // Remove the AI member's local session file. The
                    // delegation private key is not persisted on disk
                    // (it is re-derived from the operator's passphrase
                    // at issuance), so there is nothing else to clean.
                    if let Ok(project_id) = joy_core::auth::session::project_id(&root) {
                        let _ = joy_core::auth::session::remove_session(&project_id, &member_id);
                    }
                    // Remove delegation entries for this AI member from all
                    // human members in project.yaml.
                    for (_, m) in project.members.iter_mut() {
                        m.ai_delegations.remove(&member_id);
                    }
                }
            }
        }
        if project_changed {
            joy_core::store::write_yaml_preserve(&project_path, &project)?;
        }
    }

    // If no AI tools remain, update gitignore and clean up .joy/ai/
    let any_remaining = all_tools
        .iter()
        .any(|(_, id, _)| is_tool_configured(&root, id));
    if !any_remaining {
        joy_core::init::update_gitignore_block(&root, joy_core::init::GITIGNORE_BASE_ENTRIES)?;

        // Remove .joy/ai/ directory, preserving jobs/ if non-empty
        let ai_dir = joy_core::store::joy_dir(&root).join("ai");
        if ai_dir.exists() {
            let jobs_dir = ai_dir.join("jobs");
            let jobs_has_content = jobs_dir.is_dir()
                && fs::read_dir(&jobs_dir)
                    .map(|mut d| d.next().is_some())
                    .unwrap_or(false);

            if jobs_has_content {
                // Remove everything in ai/ except jobs/
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
                dprintln!(
                    "  {}{:<24} cleaned (jobs/ preserved)",
                    color::check_mark(),
                    ".joy/ai/"
                );
            } else {
                fs::remove_dir_all(&ai_dir)?;
                dprintln!("  {}{:<24} removed", color::check_mark(), ".joy/ai/");
            }
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
    dprintln!(
        "{}",
        color::footer(&format!("{} reset", color::plural(count, "tool")))
    );
    Ok(())
}

type ToolEntry = (
    &'static str,                            // display name
    &'static str,                            // id
    fn() -> bool,                            // detect: is the tool installed?
    fn(&Path, &str) -> anyhow::Result<bool>, // configure
);

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
    which("copilot") || which("gh")
}

const ALL_TOOLS: &[ToolEntry] = &[
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
fn setup_new_tools(root: &Path, passphrase: Option<&str>) -> anyhow::Result<Vec<&'static str>> {
    dprintln!("{}", color::section("AI Tools"));

    let mut configured_tools: Vec<&'static str> = Vec::new();
    let mut newly_configured = 0;

    // Load project for member registration
    let project_path = joy_core::store::joy_dir(root).join(joy_core::store::PROJECT_FILE);
    let mut project = joy_core::store::read_project(&project_path)?;
    let mut project_changed = false;

    // The acting human's identity keypair, derived lazily on the first
    // new member registration so attestations match what
    // `joy project member add` produces (JOY-011E-CF). Repeated
    // registrations within the same `joy ai init` reuse the keypair.
    let mut acting: Option<(String, joy_core::auth::IdentityKeypair)> = None;

    for (name, id, detect, configure) in ALL_TOOLS {
        if !detect() {
            continue;
        }
        let already = is_tool_configured(root, id);
        let member_id = format!("ai:{id}@joy");
        let should_register;

        if already {
            dprintln!(
                "  {}{:<24} {}",
                color::check_mark(),
                name,
                color::inactive("already configured")
            );
            configured_tools.push(*id);
            // Backfill the member entry when the tool was configured outside
            // joy ai init (e.g. by joy ai update or a copied .claude/ dir).
            should_register = true;
        } else {
            dprint!("  {}{:<24} configure? [Y/n] ", color::warn_mark(), name);
            if confirm_default_yes()? {
                configure(root, &member_id)?;
                configured_tools.push(*id);
                newly_configured += 1;
                should_register = true;
            } else {
                should_register = false;
            }
        }

        if should_register && !project.members.contains_key(&member_id) {
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

            // Derive the attesting human's keypair on first need.
            if acting.is_none() {
                let email = joy_core::vcs::default_vcs().user_email()?;
                let kp =
                    crate::commands::project::derive_acting_keypair(&project, &email, passphrase)?;
                acting = Some((email, kp));
            }
            let (attester_email, attester_kp) = acting.as_ref().unwrap();

            let signed_fields =
                joy_core::auth::attestation::signed_fields_for(&member_id, &capabilities, None);
            let attestation = joy_core::auth::attestation::sign_attestation(
                attester_email,
                attester_kp,
                signed_fields,
            );

            let mut new_member = joy_core::model::project::Member::new(capabilities);
            new_member.attestation = Some(attestation);
            project.members.insert(member_id.clone(), new_member);

            project_changed = true;
            dprintln!(
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
        dprintln!("  {}No supported AI tools detected.", color::warn_mark());
        dprintln!(
            "  {}",
            color::inactive("Supported: Claude Code, Qwen Code, Mistral Vibe, GitHub Copilot")
        );
        dprintln!(
            "  {}",
            color::inactive("Install one and re-run `joy ai init`.")
        );
    } else if newly_configured == 0 {
        dprintln!(
            "\n  {}All tools already configured. Use {} to update files.",
            color::warn_mark(),
            color::label("joy update")
        );
    }

    dprintln!();
    Ok(configured_tools)
}

/// Render the managed block (identity + instructions with workflow) for a tool's instruction file.
fn render_managed_block(member_id: &str, has_skill: bool) -> anyhow::Result<String> {
    let workflow = ai_templates::load_workflow()?;
    let joy_block = ai_templates::render_joy_block(member_id, has_skill)?;
    let instructions = ai_templates::render_instructions(&workflow)?;
    Ok(format!("{}\n\n{}", joy_block, instructions))
}

/// Render SKILL.md with workflow context.
fn render_skill() -> anyhow::Result<String> {
    let workflow = ai_templates::load_workflow()?;
    ai_templates::render_skill(&workflow).map_err(Into::into)
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
            let content = ai_templates::render_agent(agent, &workflow, tool)?;
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
    changed |= update_claude_permissions(root, member_id)?;

    Ok(changed)
}

fn update_claude_permissions(root: &Path, _member_id: &str) -> anyhow::Result<bool> {
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
    changed |= update_qwen_permissions(root, member_id)?;

    Ok(changed)
}

fn update_qwen_permissions(root: &Path, _member_id: &str) -> anyhow::Result<bool> {
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

    let json = serde_json::to_string_pretty(&settings)?;
    let changed = write_if_changed(&settings_path, &format!("{json}\n"))?;
    qprintln!("    {}.qwen/settings.json", color::check_mark());

    Ok(changed)
}

fn configure_vibe(root: &Path, member_id: &str) -> anyhow::Result<bool> {
    let vibe_dir = root.join(".vibe");
    fs::create_dir_all(&vibe_dir)?;
    clean_managed_dirs(root, &[".vibe/agents", ".vibe/skills/joy"]);
    let mut changed = false;

    // Vibe reads {root}/AGENTS.md as "Project instructions" into its system
    // prompt (see mistral-vibe vibe/core/config/harness_files). `.vibe/AGENTS.md`
    // is NOT scanned, so the file must live at the workspace root.
    let agents_md = root.join("AGENTS.md");
    changed |= update_with_joy_block(&agents_md, &render_managed_block(member_id, true)?)?;
    qprintln!("    {}AGENTS.md", color::check_mark());

    let skill_path = vibe_dir.join("skills/joy/SKILL.md");
    changed |= write_if_changed(&skill_path, &render_skill()?)?;
    qprintln!("    {}.vibe/skills/joy/SKILL.md", color::check_mark());

    let setup_path = vibe_dir.join("skills/joy/setup.md");
    changed |= write_if_changed(&setup_path, ai_templates::setup_instructions())?;
    qprintln!("    {}.vibe/skills/joy/setup.md", color::check_mark());

    changed |= generate_agents(root, "vibe", ".vibe/agents")?;

    let config_path = vibe_dir.join("config.toml");
    changed |= ensure_vibe_bash_always(&config_path)?;
    qprintln!("    {}.vibe/config.toml", color::check_mark());

    Ok(changed)
}

/// Make sure .vibe/config.toml declares `[tools.bash] permission = "always"`.
/// Creates the file if missing. If the key already exists we respect the
/// user's value and leave the file alone, so a deliberate override is
/// not clobbered on every `joy ai init`.
fn ensure_vibe_bash_always(path: &Path) -> anyhow::Result<bool> {
    use toml_edit::{value, DocumentMut, Item, Table};

    let mut doc: DocumentMut = if path.is_file() {
        fs::read_to_string(path)?.parse()?
    } else {
        DocumentMut::new()
    };

    let tools = doc
        .entry("tools")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!(".vibe/config.toml: [tools] is not a table"))?;
    tools.set_implicit(true);

    let bash = tools
        .entry("bash")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!(".vibe/config.toml: [tools.bash] is not a table"))?;

    if bash.get("permission").is_some() {
        return Ok(false);
    }
    bash["permission"] = value("always");

    write_if_changed(path, &doc.to_string())
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
    let prompt = ai_templates::render_copilot_prompt(&workflow)?;
    let prompt_path = github_dir.join("prompts/joy.prompt.md");
    changed |= write_if_changed(&prompt_path, &prompt)?;
    qprintln!("    {}.github/prompts/joy.prompt.md", color::check_mark());

    changed |= generate_agents(root, "copilot", ".github/agents")?;
    changed |= update_copilot_permissions(root, member_id)?;

    Ok(changed)
}

fn update_copilot_permissions(root: &Path, _member_id: &str) -> anyhow::Result<bool> {
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
    let changed = write_if_changed(&settings_path, &format!("{json}\n"))?;
    qprintln!("    {}.github/copilot/settings.json", color::check_mark());

    Ok(changed)
}

/// Write content to a file only if it differs.
/// Returns true if the file was changed.
fn write_if_changed(path: &Path, content: &str) -> anyhow::Result<bool> {
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
    Ok(true)
}

/// Remove the Joy-managed block from a shared file. If the file contains only
/// the Joy block (and whitespace), the file is deleted. If user content exists
/// outside the markers, that content is preserved and the file remains.
/// No-op if the file has no Joy block (never touch files Joy did not author).
fn remove_joy_block_or_file(path: &Path) -> anyhow::Result<()> {
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

fn update_gitignore(root: &Path, configured_tools: &[&str]) -> anyhow::Result<()> {
    use joy_core::init::GITIGNORE_BASE_ENTRIES;

    // Read the current managed block so we can preserve entries for AI tools
    // that aren't installed on this machine but were registered on another.
    // Without this, running `joy ai init` on a machine with fewer tools
    // silently drops entries that were committed elsewhere.
    let existing = existing_managed_block_entries(root);

    let mut entries: Vec<(&str, &str)> = GITIGNORE_BASE_ENTRIES.to_vec();

    for (tool_id, tool_entries) in TOOL_GITIGNORE_ENTRIES {
        let configured_now = configured_tools.contains(tool_id);
        let previously_recorded = tool_entries
            .iter()
            .any(|(path, _)| existing.iter().any(|line| line == *path));
        if configured_now || previously_recorded {
            entries.extend_from_slice(tool_entries);
        }
    }

    joy_core::init::update_gitignore_block(root, &entries)?;
    Ok(())
}

/// Read the lines currently inside the joy-managed `.gitignore` block.
/// Returns an empty vec if `.gitignore` is missing or has no managed block.
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

/// Scan subdirectories (max 2 levels) for nested Joy projects that lack AI tool config.
fn check_nested_projects(root: &Path) -> anyhow::Result<()> {
    let mut unconfigured: Vec<String> = Vec::new();

    // Collect installed tools to check against
    let tools: Vec<&str> = ALL_TOOLS
        .iter()
        .filter(|(_, _, detect, _)| detect())
        .map(|(_, id, _, _)| *id)
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
        dprintln!("{}", color::section("Nested Projects"));
        for path in &unconfigured {
            dprintln!("  {}{}/", color::warn_mark(), path);
        }
        dprintln!(
            "  {}",
            color::inactive("Permissions are per-project. Run `joy ai init` in each.")
        );
        dprintln!();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn arch_spec() -> &'static DocSpec {
        DOC_SPECS.iter().find(|s| s.key == "architecture").unwrap()
    }

    #[test]
    fn ensure_vibe_bash_always_creates_new_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let changed = ensure_vibe_bash_always(&path).unwrap();
        assert!(changed);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[tools.bash]"));
        assert!(content.contains("permission = \"always\""));
    }

    #[test]
    fn ensure_vibe_bash_always_adds_to_existing_unrelated_config() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "[models]\ndefault = \"mistral-large\"\n").unwrap();
        let changed = ensure_vibe_bash_always(&path).unwrap();
        assert!(changed);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("default = \"mistral-large\""));
        assert!(content.contains("permission = \"always\""));
    }

    #[test]
    fn ensure_vibe_bash_always_preserves_user_override() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "[tools.bash]\npermission = \"ask\"\n").unwrap();
        let changed = ensure_vibe_bash_always(&path).unwrap();
        assert!(!changed);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("permission = \"ask\""));
        assert!(!content.contains("\"always\""));
    }

    #[test]
    fn flag_overrides_configured_and_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let result = resolve_doc_path(
            tmp.path(),
            arch_spec(),
            Some("docs/ignored.md"),
            Some("docs/from-flag.md"),
        )
        .unwrap();
        assert_eq!(result.path, "docs/from-flag.md");
        assert_eq!(result.source, DocPathSource::Flag);
    }

    #[test]
    fn flag_trimmed_then_used() {
        let tmp = tempfile::tempdir().unwrap();
        let result = resolve_doc_path(
            tmp.path(),
            arch_spec(),
            None,
            Some("  docs/with-spaces.md  "),
        )
        .unwrap();
        assert_eq!(result.path, "docs/with-spaces.md");
        assert_eq!(result.source, DocPathSource::Flag);
    }

    #[test]
    fn configured_short_circuits_prompt() {
        let tmp = tempfile::tempdir().unwrap();
        let result =
            resolve_doc_path(tmp.path(), arch_spec(), Some("ARCHITECTURE.md"), None).unwrap();
        assert_eq!(result.path, "ARCHITECTURE.md");
        assert_eq!(result.source, DocPathSource::Configured);
    }

    #[test]
    fn empty_flag_falls_back_to_configured() {
        let tmp = tempfile::tempdir().unwrap();
        let result = resolve_doc_path(
            tmp.path(),
            arch_spec(),
            Some("ARCHITECTURE.md"),
            Some("   "),
        )
        .unwrap();
        assert_eq!(result.path, "ARCHITECTURE.md");
        assert_eq!(result.source, DocPathSource::Configured);
    }

    #[test]
    fn auto_detects_existing_candidate_without_prompt() {
        // Nothing configured, no flag, but a candidate exists on disk:
        // resolve_doc_path must take it silently rather than prompting.
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("ARCHITECTURE.md"), "stub").unwrap();
        let result = resolve_doc_path(tmp.path(), arch_spec(), None, None).unwrap();
        assert_eq!(result.path, "ARCHITECTURE.md");
        assert_eq!(result.source, DocPathSource::AutoDetected);
    }

    #[test]
    fn auto_detects_default_path_when_present() {
        // The built-in default is itself a candidate -- if the file exists at
        // the default location, that should also be auto-detected silently.
        let tmp = tempfile::tempdir().unwrap();
        let default_path = joy_core::model::project::Docs::DEFAULT_ARCHITECTURE;
        let full = tmp.path().join(default_path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(&full, "stub").unwrap();
        let result = resolve_doc_path(tmp.path(), arch_spec(), None, None).unwrap();
        assert_eq!(result.path, default_path);
        assert_eq!(result.source, DocPathSource::AutoDetected);
    }

    #[test]
    fn suggestion_is_default_when_no_candidate_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let suggestion = suggested_doc_path(tmp.path(), arch_spec());
        assert_eq!(
            suggestion,
            joy_core::model::project::Docs::DEFAULT_ARCHITECTURE
        );
    }

    #[test]
    fn suggestion_picks_first_existing_candidate() {
        let tmp = tempfile::tempdir().unwrap();
        // ARCHITECTURE.md is later in the candidate list than
        // docs/architecture/README.md; create only the latter and verify it wins.
        let dir = tmp.path().join("docs/architecture");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("README.md"), "stub").unwrap();
        let suggestion = suggested_doc_path(tmp.path(), arch_spec());
        assert_eq!(suggestion, "docs/architecture/README.md");
    }

    #[test]
    fn suggestion_skips_missing_candidates_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        // Only the very last candidate (ARCHITECTURE.md in root) exists -- it
        // should be returned despite earlier candidates being absent.
        fs::write(tmp.path().join("ARCHITECTURE.md"), "stub").unwrap();
        let suggestion = suggested_doc_path(tmp.path(), arch_spec());
        assert_eq!(suggestion, "ARCHITECTURE.md");
    }

    /// Helper: write a `.gitignore` file with a joy-managed block
    /// containing the given path lines.
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

    /// Read the joy-managed block lines from the given `.gitignore` file.
    fn read_block_lines(root: &Path) -> Vec<String> {
        existing_managed_block_entries(root)
    }

    #[test]
    fn update_gitignore_preserves_other_tools_block_entries() {
        // Drift scenario: existing block was committed from a machine with all
        // four AI tools installed; current machine only has Claude. The block
        // must keep the other tools' entries.
        let tmp = tempfile::tempdir().unwrap();
        seed_gitignore(
            tmp.path(),
            &[
                ".joy/config.yaml",
                ".joy/credentials.yaml",
                ".joy/hooks/",
                ".joy/project.defaults.yaml",
                ".claude/",
                ".qwen/",
                ".vibe/",
                "AGENTS.md",
                ".github/copilot-instructions.md",
                ".github/copilot/",
                ".github/agents/",
                ".github/prompts/",
            ],
        );

        update_gitignore(tmp.path(), &["claude"]).unwrap();

        let lines = read_block_lines(tmp.path());
        assert!(lines.iter().any(|l| l == ".claude/"));
        assert!(lines.iter().any(|l| l == ".qwen/"));
        assert!(lines.iter().any(|l| l == ".vibe/"));
        assert!(lines.iter().any(|l| l == "AGENTS.md"));
        assert!(lines.iter().any(|l| l == ".github/copilot-instructions.md"));
        assert!(lines.iter().any(|l| l == ".github/copilot/"));
        assert!(lines.iter().any(|l| l == ".github/agents/"));
        assert!(lines.iter().any(|l| l == ".github/prompts/"));
    }

    #[test]
    fn update_gitignore_adds_newly_configured_tool() {
        // Existing block has only Claude; init now sees Claude + Qwen
        // configured. Both tools' entries must be present afterwards.
        let tmp = tempfile::tempdir().unwrap();
        seed_gitignore(
            tmp.path(),
            &[
                ".joy/config.yaml",
                ".joy/credentials.yaml",
                ".joy/hooks/",
                ".joy/project.defaults.yaml",
                ".claude/",
            ],
        );

        update_gitignore(tmp.path(), &["claude", "qwen"]).unwrap();

        let lines = read_block_lines(tmp.path());
        assert!(lines.iter().any(|l| l == ".claude/"));
        assert!(lines.iter().any(|l| l == ".qwen/"));
    }

    #[test]
    fn update_gitignore_does_not_resurrect_unrelated_lines() {
        // Lines inside the block that don't correspond to a known tool entry
        // must NOT be carried over -- the additive policy only preserves
        // entries that Joy itself manages.
        let tmp = tempfile::tempdir().unwrap();
        seed_gitignore(
            tmp.path(),
            &[
                ".joy/config.yaml",
                ".joy/credentials.yaml",
                ".joy/hooks/",
                ".joy/project.defaults.yaml",
                ".claude/",
                "user-injected-line.txt",
            ],
        );

        update_gitignore(tmp.path(), &["claude"]).unwrap();

        let lines = read_block_lines(tmp.path());
        assert!(lines.iter().any(|l| l == ".claude/"));
        assert!(!lines.iter().any(|l| l == "user-injected-line.txt"));
    }

    #[test]
    fn update_gitignore_creates_block_from_scratch_when_absent() {
        // No existing `.gitignore`: init writes a fresh block with base
        // entries plus configured tools. Nothing else.
        let tmp = tempfile::tempdir().unwrap();
        update_gitignore(tmp.path(), &["claude"]).unwrap();
        let lines = read_block_lines(tmp.path());
        assert!(lines.iter().any(|l| l == ".joy/credentials.yaml"));
        assert!(lines.iter().any(|l| l == ".claude/"));
        assert!(!lines.iter().any(|l| l == ".qwen/"));
    }

    #[test]
    fn existing_managed_block_entries_returns_empty_when_no_block() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".gitignore"), "*.log\nnode_modules/\n").unwrap();
        assert!(existing_managed_block_entries(tmp.path()).is_empty());
    }

    #[test]
    fn existing_managed_block_entries_returns_empty_when_no_gitignore() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(existing_managed_block_entries(tmp.path()).is_empty());
    }

    #[test]
    fn update_with_joy_block_creates_agents_md() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("AGENTS.md");
        let changed = update_with_joy_block(&path, "hello world").unwrap();
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
        update_with_joy_block(&path, "new content").unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("user header"));
        assert!(content.contains("new content"));
        assert!(!content.contains("old"));
        assert!(content.trim_end().ends_with("user footer"));
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

    #[test]
    fn remove_joy_block_leaves_unmanaged_file_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("AGENTS.md");
        fs::write(&path, "pure user content\n").unwrap();
        remove_joy_block_or_file(&path).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "pure user content\n");
    }
}

#[derive(serde::Serialize)]
struct AiStatusPayload {
    check: bool,
    tools: Vec<AiToolEntry>,
    has_issues: bool,
}

#[derive(serde::Serialize)]
struct AiToolEntry {
    name: String,
    id: String,
    installed: bool,
    configured: bool,
    stale: bool,
}
