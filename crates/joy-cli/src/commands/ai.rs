// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::fs;
use std::io::Write;
use std::path::Path;

use joy_ai::ai_setup::{
    is_tool_configured, is_tool_stale, plan_member_reset, remove_joy_block_or_file,
    remove_legacy_ai_artifacts, untrack_gitignored_tool_files, update_gitignore, MemberResetPlan,
    TOOLS as ALL_TOOLS,
};

use std::sync::atomic::AtomicBool;

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

const VISION_TEMPLATE: &str = include_str!("../../docs/VISION.md");
const ARCHITECTURE_TEMPLATE: &str = include_str!("../../docs/ARCHITECTURE.md");
const CONTRIBUTING_TEMPLATE: &str = include_str!("../../docs/CONTRIBUTING.md");

#[derive(clap::Args)]
#[command(
    after_help = "For chat-only or otherwise undetected AI tools (e.g. Copilot Chat in VS Code, Cursor's built-in chat), register the member manually:\n  joy project member add ai:<name>@joy\nthen issue a delegation token with `joy auth token add ai:<name>@joy`."
)]
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
    /// Read the AI operational guide (CLI reference for AI assistants)
    Tutorial(AiTutorialArgs),
}

#[derive(clap::Args)]
struct AiTutorialArgs {
    /// Browse the tutorial via a chapter / subchapter menu (TTY only).
    #[arg(short = 'i', long)]
    interactive: bool,
}

#[derive(clap::Args, Default)]
struct InitArgs {
    /// Path to the architecture doc (e.g. ARCHITECTURE.md). Skips the prompt.
    #[arg(long, value_hint = clap::ValueHint::FilePath)]
    architecture: Option<String>,

    /// Path to the vision doc (e.g. VISION.md). Skips the prompt.
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

    /// Read the passphrase from a single line on stdin. See
    /// `joy auth --help` for the rationale; same flag, same semantics.
    #[arg(long = "passphrase-stdin")]
    passphrase_stdin: bool,

    /// Only set up a specific tool (claude, qwen, vibe, copilot) — even
    /// when it is not auto-detected. Skips the docs prompts.
    #[arg(long)]
    tool: Option<String>,
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

    /// Read the passphrase from a single line on stdin.
    #[arg(long = "passphrase-stdin")]
    passphrase_stdin: bool,
}

pub fn run(args: AiArgs) -> anyhow::Result<()> {
    match args.command {
        AiCommand::Init(a) => ai_init(a),
        AiCommand::Reset(a) => reset(a),
        AiCommand::Rotate(a) => crate::commands::auth::run_ai_rotate(
            &a.member,
            a.passphrase.as_deref(),
            a.passphrase_stdin,
        ),
        AiCommand::Tutorial(a) => ai_tutorial(a),
    }
}

// The canonical AI Tutorial lives at docs/ai/Tutorial.md in the repo
// root. We ship an in-crate copy at crates/joy-cli/docs/ai/Tutorial.md
// because `cargo package` builds the crate in isolation and cannot
// reach files outside the crate root. The two files must stay
// byte-identical; `just sync-tutorial` refreshes the copy and the
// unit test below catches drift. Same pattern as the user-facing
// tutorial, see JOY-017F-FD.
const AI_TUTORIAL: &str = include_str!("../../docs/ai/Tutorial.md");

fn ai_tutorial(args: AiTutorialArgs) -> anyhow::Result<()> {
    // `joy ai tutorial` is targeted at AI consumption, so it opts out of
    // the pager unconditionally. Even when stdout is a TTY (an AI tool
    // runner that wires stdio through a PTY), spawning `less` would
    // block the runner waiting for keyboard input. A human who wants
    // scrolling can pipe to their own pager.
    joy_core::tutorial::run_markdown(AI_TUTORIAL, args.interactive, false)?;
    Ok(())
}

#[cfg(test)]
#[test]
fn in_crate_ai_tutorial_matches_canonical() {
    let canonical = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/ai/Tutorial.md"
    ));
    assert_eq!(
        canonical, AI_TUTORIAL,
        "crates/joy-cli/docs/AiTutorial.md is out of sync with \
         docs/ai/Tutorial.md. Run `just sync-tutorial`."
    );
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

    let bootstrapped_passphrase =
        ensure_human_auth_initialized(&root, args.passphrase.as_deref(), args.passphrase_stdin)?;
    let effective_passphrase = bootstrapped_passphrase
        .as_deref()
        .or(args.passphrase.as_deref());
    if let Some(filter) = args.tool.as_deref() {
        if !ALL_TOOLS.iter().any(|(_, id, _, _)| *id == filter) {
            let valid: Vec<&str> = ALL_TOOLS.iter().map(|(_, id, _, _)| *id).collect();
            anyhow::bail!("unknown tool: {filter}\nknown tools: {}", valid.join(", "));
        }
    } else {
        check_docs(&root, &args)?;
    }
    let configured_tools = setup_new_tools(
        &root,
        effective_passphrase,
        args.passphrase_stdin,
        args.tool.as_deref(),
    )?;
    update_gitignore(&root, &configured_tools)?;
    untrack_gitignored_tool_files(&root);
    let removed_legacy = remove_legacy_ai_artifacts(&root);
    if !removed_legacy.is_empty() {
        dprintln!(
            "{}",
            color::warning(&format!(
                "Removed {} legacy pre-ADR-024 artefact(s): {}. Commit the removal to finish.",
                removed_legacy.len(),
                removed_legacy.join(", ")
            ))
        );
    }
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

/// Ensure the acting human has authentication initialised before AI tool
/// setup runs. Without a registered public key, member registration will
/// fail partway through `setup_new_tools()`, leaving doc templates and AI
/// tool configs written but no members attested. Detecting and resolving
/// here keeps the flow in one pass.
///
/// Returns `Some(passphrase)` if auth was just bootstrapped, so the caller
/// can pass it forward to subsequent operations such as AI member
/// attestations in `setup_new_tools` without re-prompting. Returns `None`
/// if auth was already initialised.
fn ensure_human_auth_initialized(
    root: &Path,
    passphrase: Option<&str>,
    passphrase_stdin: bool,
) -> anyhow::Result<Option<String>> {
    let project_path = joy_core::store::joy_dir(root).join(joy_core::store::PROJECT_FILE);
    let project = joy_core::store::read_project(&project_path)?;
    let email = joy_core::vcs::default_vcs().user_email()?;
    // Resolve the member honoring the project's privacy mode. In anonymous
    // mode (ADR-042) the member map is keyed by an opaque id, not the
    // cleartext e-mail, so a direct `members.get(&email)` would spuriously
    // report the founder as unregistered right after `joy init --anonymous`.
    let member_key =
        joy_core::privacy::member_key_for_email(&project, &email).ok_or_else(|| {
            anyhow::anyhow!(
                "{} is not a registered project member. Run `joy project member add {}` first.",
                email,
                email
            )
        })?;
    let member = project.member_by_key(&member_key).unwrap();
    if member.verify_key.is_some() {
        return Ok(None);
    }

    dprintln!("{}", color::section("Authentication"));
    dprintln!(
        "{}",
        color::inactive(
            "AI tool members are attested with your project key, so authentication is required before registration."
        )
    );
    let bootstrapped = crate::commands::auth::run_init(passphrase, passphrase_stdin, None, false)?;
    dprintln!();
    Ok(Some(bootstrapped))
}

/// Check if a tool's generated files are up to date by re-rendering expected
/// content and comparing against on-disk files.
/// Look up an ALL_TOOLS entry by id; exposed for the update registry.
pub(crate) fn tool_display_name(id: &str) -> Option<&'static str> {
    ALL_TOOLS
        .iter()
        .find(|(_, eid, _, _)| *eid == id)
        .map(|(name, _, _, _)| *name)
}

pub(crate) fn tool_ids() -> &'static [&'static str] {
    &["claude", "qwen", "vibe", "copilot"]
}

pub(crate) fn is_tool_installed(id: &str) -> bool {
    ALL_TOOLS
        .iter()
        .find(|(_, eid, _, _)| *eid == id)
        .map(|(_, _, detect, _)| detect())
        .unwrap_or(false)
}

pub(crate) fn is_tool_configured_pub(root: &Path, id: &str) -> bool {
    is_tool_configured(root, id)
}

pub(crate) fn is_tool_stale_pub(root: &Path, id: &str, member_id: &str) -> anyhow::Result<bool> {
    is_tool_stale(root, id, member_id).map_err(Into::into)
}

/// Sync the joy-managed `.gitignore` block with the entries needed for
/// every currently configured AI tool.
pub(crate) fn sync_gitignore_for_configured_tools(root: &Path) -> anyhow::Result<()> {
    let configured: Vec<&'static str> = ALL_TOOLS
        .iter()
        .filter(|(_, id, _, _)| is_tool_configured(root, id))
        .map(|(_, id, _, _)| *id)
        .collect();
    update_gitignore(root, &configured).map_err(Into::into)
}

/// Refresh a single AI tool by id. Used by the update registry.
pub(crate) fn refresh_tool_by_id(root: &Path, id: &str, member_id: &str) -> anyhow::Result<bool> {
    let entry = ALL_TOOLS
        .iter()
        .find(|(_, eid, _, _)| *eid == id)
        .ok_or_else(|| anyhow::anyhow!("unknown ai tool id: {id}"))?;
    let configure = entry.3;
    QUIET.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut report = |line: String| qprintln!("    {}{}", color::check_mark(), line);
    let changed = configure(root, member_id, &mut report);
    QUIET.store(false, std::sync::atomic::Ordering::Relaxed);
    changed.map_err(Into::into)
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
            "VISION.md",
            "docs/dev/vision/README.md",
            "docs/vision/README.md",
            "docs/vision.md",
        ],
        template: VISION_TEMPLATE,
    },
    DocSpec {
        key: "architecture",
        label: "Architecture",
        purpose: "technical stack and structure",
        default_path: joy_core::model::project::Docs::DEFAULT_ARCHITECTURE,
        candidates: &[
            "ARCHITECTURE.md",
            "docs/dev/architecture/README.md",
            "docs/architecture/README.md",
            "docs/architecture.md",
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
                let input = ask_line()?.unwrap_or_default();
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
            // The doc's purpose is explained before the path prompt now
            // (JOY-01C9-A0), so this only needs to offer the action.
            dprint!("    Create {} template? [Y/n] ", name);
            let input = ask_line()?.unwrap_or_default();
            let trimmed = input.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("y") {
                if let Some(parent) = full.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&full, spec.template)?;
                if let Ok(rel) = full.strip_prefix(root) {
                    let rel_str = rel.to_string_lossy().into_owned();
                    joy_core::git_ops::auto_git_add(root, &[&rel_str]);
                }
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
        let rel = format!(
            "{}/{}",
            joy_core::store::JOY_DIR,
            joy_core::store::PROJECT_FILE
        );
        joy_core::git_ops::auto_git_add(root, &[&rel]);
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

    // Explain what the document is for BEFORE asking for its path, so the user
    // knows what to point at while choosing -- not only afterwards on the
    // "Create template?" line (JOY-01C9-A0).
    dprintln!(
        "    {}",
        color::inactive(&format!(
            "The {} doc helps AI understand your {}.",
            spec.label, spec.purpose
        ))
    );
    dprint!("    {} doc path [{}]: ", spec.label, suggestion);
    let input = ask_line()?.unwrap_or_default();
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

    // Collect the local config files that exist for the selected tools.
    let mut to_remove: Vec<(&str, &str)> = Vec::new();
    for (name, _, paths) in &tools {
        for path in *paths {
            let full = root.join(path);
            if full.exists() {
                to_remove.push((name, path));
            }
        }
    }

    // Compute the shared project.yaml changes, if any. Removing a member is a
    // mutation of versioned, shared state, so it is deliberately kept separate
    // from deleting per-developer config files: a member is only ever removed
    // when it is orphaned (no operator still delegates it), never merely because
    // the local config happens to be gone (JOY-01CD-D5).
    let project_path = joy_core::store::joy_dir(&root).join(joy_core::store::PROJECT_FILE);
    let mut project = joy_core::store::read_project(&project_path).ok();
    let caller_key = project.as_ref().and_then(|_| {
        joy_core::identity::resolve_identity(&root)
            .ok()
            .map(|id| id.member.id().to_string())
    });
    let mut plans: Vec<MemberResetPlan> = Vec::new();
    if let Some(ref p) = project {
        // Canonical tool members (ai:<tool>@joy).
        let member_ids: Vec<String> = tools
            .iter()
            .map(|(_, id, _)| format!("ai:{id}@joy"))
            .collect();
        for member_id in &member_ids {
            if let Some(plan) = plan_member_reset(p, &root, member_id, caller_key.as_deref()) {
                if plan.drop_caller_delegation || plan.remove_member {
                    plans.push(plan);
                }
            }
        }
    }

    if to_remove.is_empty() && plans.is_empty() {
        dprintln!("{}No AI tool configurations found.", color::check_mark());
        return Ok(());
    }

    // Show the full plan before touching anything.
    dprintln!("{}", color::header("AI Reset"));
    dprintln!();
    if !to_remove.is_empty() {
        dprintln!("Will remove (local config):");
        for (name, path) in &to_remove {
            dprintln!("  {}{:<24} {}", color::cross_mark(), name, path);
        }
    }
    if !plans.is_empty() {
        if !to_remove.is_empty() {
            dprintln!();
        }
        dprintln!("Will change project.yaml (shared, versioned):");
        for plan in &plans {
            if plan.remove_member {
                let warn = if plan.active_session {
                    "  [has an ACTIVE session]"
                } else {
                    ""
                };
                dprintln!(
                    "  {}{:<24} remove member (orphaned){}",
                    color::cross_mark(),
                    plan.member_id,
                    warn
                );
            } else {
                dprintln!(
                    "  {}{:<24} drop your delegation ({} other kept)",
                    color::cross_mark(),
                    plan.member_id,
                    plan.other_delegators
                );
            }
        }
    }

    // Any change here needs consent. In a non-interactive run we refuse rather
    // than silently mutate shared state; the caller must pass --force.
    if !args.force {
        if !crate::prompt::is_interactive() {
            anyhow::bail!(
                "joy ai reset would delete files or change project.yaml but stdin/stdout is not a \
                 terminal; re-run with --force to confirm non-interactively"
            );
        }
        dprintln!();
        dprint!("Proceed? [y/N] ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            dprintln!("Aborted.");
            return Ok(());
        }
    }

    // 1) Remove local config files.
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

    // 2) Apply the project.yaml changes: drop the caller's own delegation, and
    // remove the member only when it is thereby orphaned.
    if let Some(ref mut p) = project {
        let mut project_changed = false;
        for plan in &plans {
            if plan.drop_caller_delegation {
                if let Some(ck) = caller_key.as_deref() {
                    if let Some(m) = p.member_by_key_mut(ck) {
                        if m.ai_delegations.remove(&plan.member_id).is_some() {
                            project_changed = true;
                        }
                    }
                }
            }
            if plan.remove_member {
                if p.remove_member(&plan.member_id).is_some() {
                    project_changed = true;
                    // Drop the local session so a removed member cannot keep
                    // acting from a cached credential.
                    if let Ok(project_id) = joy_core::auth::session::project_id(&root) {
                        let _ =
                            joy_core::auth::session::remove_session(&project_id, &plan.member_id);
                    }
                    // Defensive: clear any delegation still pointing at the now
                    // removed member so no entry is left dangling.
                    let member_keys: Vec<String> = p.member_keys().cloned().collect();
                    for k in &member_keys {
                        if let Some(m) = p.member_by_key_mut(k) {
                            m.ai_delegations.remove(&plan.member_id);
                        }
                    }
                    dprintln!(
                        "  {}{:<24} member removed",
                        color::check_mark(),
                        plan.member_id
                    );
                } else if plan.drop_caller_delegation {
                    dprintln!(
                        "  {}{:<24} delegation removed (member kept)",
                        color::check_mark(),
                        plan.member_id
                    );
                }
            } else if plan.drop_caller_delegation {
                dprintln!(
                    "  {}{:<24} delegation removed (member kept)",
                    color::check_mark(),
                    plan.member_id
                );
            }
        }
        if project_changed {
            joy_core::store::write_yaml_preserve(&project_path, p)?;
            let rel = format!(
                "{}/{}",
                joy_core::store::JOY_DIR,
                joy_core::store::PROJECT_FILE
            );
            joy_core::git_ops::auto_git_add(&root, &[&rel]);
        }
    }

    // If no AI members remain on the project, shrink the gitignore block
    // and clean up .joy/ai/. Membership is the repo-portable signal; the
    // machine-local marker files must not decide committed content
    // (JOY-0264-89).
    let any_remaining = joy_ai::ai_setup::has_ai_member(&root);
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
    let summary = format!("{} reset", color::plural(count, "tool"));
    dprintln!("{}", color::footer(&summary));
    Ok(())
}

/// Set up only NEW (not yet configured) tools. Returns list of all configured tool IDs.
fn setup_new_tools(
    root: &Path,
    passphrase: Option<&str>,
    passphrase_stdin: bool,
    only: Option<&str>,
) -> anyhow::Result<Vec<&'static str>> {
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
        match only {
            // an explicit --tool wins over auto-detection: the operator
            // (or the app's settings toggle) asked for exactly this one
            Some(filter) => {
                if *id != filter {
                    continue;
                }
            }
            None => {
                if !detect() {
                    continue;
                }
            }
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
                let mut report = |line: String| qprintln!("    {}{}", color::check_mark(), line);
                configure(root, &member_id, &mut report)?;
                configured_tools.push(*id);
                newly_configured += 1;
                should_register = true;
            } else {
                should_register = false;
            }
        }

        if should_register && !project.has_member_key(&member_id) {
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
                let kp = crate::commands::project::derive_acting_keypair(
                    &project,
                    &email,
                    passphrase,
                    passphrase_stdin,
                )?;
                // Reference the attester by their on-disk member key so that in
                // anonymous mode (ADR-042) the stored attester is the opaque id,
                // never the cleartext e-mail. This keeps the committed
                // project.yaml e-mail-free and lets verification resolve the
                // attester via the member map, which is keyed by that id.
                let attester_id =
                    joy_core::privacy::member_key_for_email(&project, &email).unwrap_or(email);
                acting = Some((attester_id, kp));
            }
            let (attester_id, attester_kp) = acting.as_ref().unwrap();

            let signed_fields =
                joy_core::auth::attestation::signed_fields_for(&member_id, &capabilities, None);
            let attestation = joy_core::auth::attestation::sign_attestation(
                attester_id,
                attester_kp,
                signed_fields,
            );

            let mut new_member = joy_core::model::project::Member::new(capabilities);
            new_member.attestation = Some(attestation);
            // Record the ACP adapter on the member (JI-0164): the adapter lives
            // in project.yaml now, so the platform can route turns without a
            // per-member agent file.
            new_member.adapter = joy_ai::naming::tool_adapter(id).map(String::from);
            project.register_member(&member_id, new_member)?;

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
        let rel = format!(
            "{}/{}",
            joy_core::store::JOY_DIR,
            joy_core::store::PROJECT_FILE
        );
        joy_core::git_ops::auto_git_add(root, &[&rel]);
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

/// Read one answer line, or None for "take the default" when no terminal
/// is attached (GUI starts must never block on stdin).
fn ask_line() -> anyhow::Result<Option<String>> {
    if !crate::prompt::is_interactive() {
        return Ok(None);
    }
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(Some(input))
}

fn confirm_default_yes() -> anyhow::Result<bool> {
    let Some(input) = ask_line()? else {
        return Ok(true);
    };
    let trimmed = input.trim();
    Ok(trimmed.is_empty() || trimmed.eq_ignore_ascii_case("y"))
}

/// Untrack AI tool files that are listed in the joy-managed `.gitignore`
/// block but were committed before that entry existed (JOY-019E-3A).
///
/// True if any dead pre-ADR-024 artefact (see
/// [`joy_core::init::LEGACY_AI_ARTIFACTS`]) still exists on disk under
/// `root`. Read-only; drives the `joy update --check` row.
pub(crate) fn legacy_ai_artifacts_present(root: &Path) -> bool {
    joy_core::init::LEGACY_AI_ARTIFACTS
        .iter()
        .any(|rel| root.join(rel).exists())
}

/// Remove the dead pre-ADR-024 artefacts listed in
/// [`joy_core::init::LEGACY_AI_ARTIFACTS`]. Hard removal: tracked paths are
/// deleted from the working tree and the deletion is staged via `git rm`; a
/// path that is present but untracked (a local-only leftover) is removed
/// from disk directly. Idempotent -- returns the relative paths actually
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

#[cfg(test)]
mod tests {
    use super::*;

    fn arch_spec() -> &'static DocSpec {
        DOC_SPECS.iter().find(|s| s.key == "architecture").unwrap()
    }

    // --- JOY-01CD-D5: joy ai reset member-removal safety ---
    //
    // A member is removed only when it is orphaned (no operator delegates it);
    // while any delegation remains the shared entry is kept and only the
    // caller's own delegation is dropped. The decision must not depend on local
    // config files. `project_id` on a non-existent root fails, so `active_session`
    // is always false in these unit tests (no session on disk).

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
        // Root ARCHITECTURE.md is the first candidate but is absent here; only
        // the lower-priority docs/architecture/README.md exists, so it wins.
        let dir = tmp.path().join("docs/architecture");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("README.md"), "stub").unwrap();
        let suggestion = suggested_doc_path(tmp.path(), arch_spec());
        assert_eq!(suggestion, "docs/architecture/README.md");
    }

    #[test]
    fn suggestion_skips_missing_candidates_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        // Only the last candidate (docs/architecture.md) exists -- it should be
        // returned despite all earlier candidates (root ARCHITECTURE.md and the
        // nested README paths) being absent.
        let docs = tmp.path().join("docs");
        fs::create_dir_all(&docs).unwrap();
        fs::write(docs.join("architecture.md"), "stub").unwrap();
        let suggestion = suggested_doc_path(tmp.path(), arch_spec());
        assert_eq!(suggestion, "docs/architecture.md");
    }

    #[test]
    fn legacy_ai_artifacts_present_reflects_disk_state() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        assert!(!legacy_ai_artifacts_present(root));
        fs::create_dir_all(root.join(".joy/capabilities")).unwrap();
        fs::write(root.join(".joy/capabilities/plan.md"), "old").unwrap();
        assert!(legacy_ai_artifacts_present(root));
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
