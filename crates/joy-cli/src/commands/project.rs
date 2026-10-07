// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::{bail, Result};
use clap::Args;

use joy_core::auth::IdentityKeypair;
use joy_core::context::Context;
use joy_core::guard::Action;
use joy_core::model::item::Capability;
use joy_core::model::project::{CapabilityConfig, Member, MemberCapabilities, PrivacyMode};
use joy_core::model::Project;
// The get/set core (key catalogue, value tree, per-key write rules and
// YAML pruning) lives in joy-core::project_meta so the desktop app and
// the platform server share it; this file keeps the CLI-only pieces
// (clap args, editor flows, printing, privacy switch, member flows).
use joy_core::project_meta::{
    current_scalar_value, is_list_key, project_value_tree, scalar_str, set_value,
    value_as_optional_string, wildcard_prefix, LIST_KEYS, PROJECT_KEYS,
};
use joy_core::store;
use joy_core::version_files::{
    version_files_add, version_files_get, version_files_rm, version_files_set, AddOutcome,
};

use crate::color;

/// Parse an interaction-level argument value; an empty string means "clear"
/// (`None`), anything else must be one of the three level names.
fn parse_level(s: &str) -> Result<joy_core::model::config::InteractionLevel> {
    s.trim()
        .parse()
        .map_err(|e: String| anyhow::anyhow!("{}", e))
}

/// The capabilities named on the command line: none, `all`, or a list
/// (given as separate words or with commas, both read the same).
enum NamedCapabilities {
    Unsaid,
    All,
    List(Vec<Capability>),
}

fn parse_capabilities(words: &[String]) -> Result<NamedCapabilities> {
    let words: Vec<&str> = words
        .iter()
        .map(|w| w.trim())
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return Ok(NamedCapabilities::Unsaid);
    }
    if words == ["all"] {
        return Ok(NamedCapabilities::All);
    }
    let mut list = Vec::new();
    for word in words {
        let cap: Capability = word.parse().map_err(|e: String| anyhow::anyhow!("{}", e))?;
        if !list.contains(&cap) {
            list.push(cap);
        }
    }
    Ok(NamedCapabilities::List(list))
}

fn specific(list: &[Capability]) -> MemberCapabilities {
    MemberCapabilities::Specific(
        list.iter()
            .map(|cap| (*cap, CapabilityConfig::default()))
            .collect(),
    )
}

/// One column of `member show` for an AI member: what one side allows.
fn may_column(may: Option<&joy_core::auth::grants::Effective>, cap: &Capability) -> &'static str {
    match may {
        Some(may) if may.allows(cap) => "x",
        Some(_) => "-",
        None => "",
    }
}

#[derive(Args)]
pub struct ProjectArgs {
    /// Set the project name
    #[arg(long)]
    name: Option<String>,

    /// Set the project description
    #[arg(long)]
    description: Option<String>,

    /// Set the project language (e.g. en, de, fr)
    #[arg(long)]
    language: Option<String>,

    #[command(subcommand)]
    command: Option<ProjectCommand>,
}

#[derive(clap::Subcommand)]
enum ProjectCommand {
    /// Get a project value: name|acronym|description|language|created
    Get(GetArgs),
    /// Set a project value: name|acronym|description|language
    Set(SetArgs),
    /// Manage project members
    Member(MemberArgs),
}

#[derive(clap::Args)]
struct GetArgs {
    /// Project key (e.g. `name`, `docs.architecture`). A trailing `.*`
    /// lists every leaf under that prefix.
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(complete_project_key))]
    key: String,

    /// Append a one-line semantic description to each value. Same flag
    /// and shape as `joy config get --describe`.
    #[arg(long)]
    describe: bool,
}

#[derive(clap::Args)]
struct SetArgs {
    /// Project key
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(complete_project_key))]
    key: String,
    /// Value to set. For list-typed keys (release.version-files) a
    /// comma-separated list replaces the whole list; an empty string
    /// clears it. Omit when using --add or --rm.
    value: Option<String>,
    /// Append a single entry to a list-typed key. Idempotent: warns
    /// and exits 0 if the entry is already configured.
    #[arg(long, conflicts_with = "rm", conflicts_with = "value")]
    add: Option<String>,
    /// Remove a single entry from a list-typed key. Errors if the
    /// entry is not configured.
    #[arg(long, conflicts_with = "value")]
    rm: Option<String>,
    /// Editor command to use when VALUE is omitted (overrides config /
    /// $VISUAL / $EDITOR). Mirrors `joy comment`.
    #[arg(long)]
    editor: Option<String>,
}

#[derive(clap::Args)]
struct MemberArgs {
    #[command(subcommand)]
    command: Option<MemberCommand>,
}

#[derive(clap::Subcommand)]
enum MemberCommand {
    /// Show member details
    Show(MemberShowArgs),
    /// Add a project member
    Add(MemberAddArgs),
    /// Edit what a member may do
    Edit(MemberEditArgs),
    /// Remove a project member
    Rm(MemberRmArgs),
    /// Erase a member's e-mail/name from the encrypted members.yaml (GDPR
    /// Art. 17), keeping the opaque id and audit trail. Anonymous mode only.
    Erase(MemberEraseArgs),
}

#[derive(clap::Args)]
struct MemberEraseArgs {
    /// Member to erase (e-mail or opaque id).
    id: String,
}

#[derive(clap::Args)]
struct MemberShowArgs {
    /// Member: a person's address, or an AI member's name
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_member))]
    id: String,
}

#[derive(clap::Args)]
struct MemberAddArgs {
    /// Member: a person's address, or a name for an AI member
    id: String,

    /// Capabilities, or `all`. Default: everything but manage and delete.
    #[arg(short = 'c', long, num_args = 1.., value_delimiter = ',')]
    capabilities: Vec<String>,

    /// The most an AI member may do on its own: proposing, confirmed or autonomous.
    #[arg(long, value_name = "LEVEL")]
    level: Option<String>,

    /// The tool that runs an AI member. Default: the tool the name names.
    #[arg(long)]
    adapter: Option<String>,

    /// The model an AI member runs on. Default: the tool's own.
    #[arg(long)]
    model: Option<String>,

    /// What an AI member is for.
    #[arg(long)]
    description: Option<String>,

    /// Issue a delegation token for an AI member right away.
    #[arg(long = "with-token")]
    with_token: bool,
}

#[derive(clap::Args)]
struct MemberRmArgs {
    /// Member: a person's address, or an AI member's name
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_member))]
    id: String,
}

#[derive(clap::Args)]
struct MemberEditArgs {
    /// Member: a person's address, or an AI member's name
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_member))]
    id: String,

    /// Replace the capabilities, or `all`.
    #[arg(short = 'c', long, num_args = 1.., value_delimiter = ',', conflicts_with_all = ["add_capability", "rm_capability"])]
    capabilities: Vec<String>,

    /// Grant one capability, keeping the rest (repeatable).
    #[arg(long = "add-capability", value_name = "CAP")]
    add_capability: Vec<String>,

    /// Revoke one capability, keeping the rest (repeatable).
    #[arg(long = "rm-capability", value_name = "CAP")]
    rm_capability: Vec<String>,

    /// The most an AI member may do on its own: proposing, confirmed or autonomous.
    #[arg(long, value_name = "LEVEL")]
    level: Option<String>,

    /// Change what the project allows an AI member, not what you allow it yourself.
    #[arg(long)]
    project: bool,

    /// The model an AI member runs on (with --project).
    #[arg(long)]
    model: Option<String>,

    /// What an AI member is for (with --project).
    #[arg(long)]
    description: Option<String>,
}

pub fn run(args: ProjectArgs) -> Result<()> {
    let mut ctx = Context::load()?;

    let mut project: Project = store::load_project(&ctx.root)?;

    match args.command {
        Some(ProjectCommand::Get(a)) => {
            return get_value(&ctx.root, &project, &a.key, a.describe);
        }
        Some(ProjectCommand::Set(a)) => {
            crate::auth_gate::enforce(&mut ctx, &Action::ManageProject, "project")?;
            return set_command(&ctx, &mut project, a);
        }
        Some(ProjectCommand::Member(a)) => {
            return run_member(a, &mut project, &mut ctx);
        }
        None => {}
    }

    // Legacy flag-based editing
    let is_edit = args.name.is_some() || args.description.is_some() || args.language.is_some();

    if is_edit {
        crate::auth_gate::enforce(&mut ctx, &Action::ManageProject, "project")?;
        if let Some(name) = args.name {
            project.name = name;
        }
        if let Some(description) = args.description {
            project.description = if description.is_empty() {
                None
            } else {
                Some(description)
            };
        }
        if let Some(language) = args.language {
            project.language = language;
        }
        store::save_project(&ctx.root, &project)?;
        println!("Project updated.");
        let log_user = ctx.log_user();
        joy_core::git_ops::auto_git_post_command(&ctx.root, "project edit", &log_user);
    }

    if crate::output::is_json() {
        return crate::output::emit(&project);
    }
    show_project(&project, &ctx.root);
    Ok(())
}

fn get_value(root: &std::path::Path, project: &Project, key: &str, describe: bool) -> Result<()> {
    let tree = project_value_tree(root, project);

    // Wildcard form: `docs.*` lists every leaf under that prefix.
    // Mirrors `joy config get <prefix>.*` (JOY-0187-D0).
    if let Some(prefix) = wildcard_prefix(key) {
        return get_wildcard(&tree, key, prefix, describe);
    }

    if !PROJECT_KEYS.contains(&key) {
        anyhow::bail!(
            "unknown key: {key}\nknown keys: {}",
            PROJECT_KEYS.join(", ")
        );
    }

    if is_list_key(key) {
        return get_list_value(root, key, describe);
    }

    let value = joy_core::model::config::flatten_under(&tree, "");
    let scalar = value.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());

    if crate::output::is_json() {
        #[derive(serde::Serialize)]
        struct GetPayload<'a> {
            key: &'a str,
            value: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            description: Option<String>,
        }
        let description = if describe {
            scalar
                .as_ref()
                .and_then(|v| joy_core::model::project::describe_value(key, v))
        } else {
            None
        };
        return crate::output::emit(GetPayload {
            key,
            value: scalar.as_ref().and_then(value_as_optional_string),
            description,
        });
    }

    let Some(value) = scalar else {
        std::process::exit(1);
    };

    let suffix = if describe {
        joy_core::model::project::describe_value(key, &value)
            .map(|d| format!("  {} {}", color::inactive("-"), color::inactive(&d)))
            .unwrap_or_default()
    } else {
        String::new()
    };

    match &value {
        serde_json::Value::Null => std::process::exit(1),
        serde_json::Value::String(s) => println!("{s}{suffix}"),
        other => println!("{other}{suffix}"),
    }
    Ok(())
}

fn get_wildcard(tree: &serde_json::Value, key: &str, prefix: &str, describe: bool) -> Result<()> {
    let leaves = joy_core::model::config::flatten_under(tree, prefix);

    if crate::output::is_json() {
        #[derive(serde::Serialize)]
        struct Entry {
            key: String,
            value: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            description: Option<String>,
        }
        #[derive(serde::Serialize)]
        struct Payload<'a> {
            key: &'a str,
            entries: Vec<Entry>,
        }
        let entries = leaves
            .into_iter()
            .map(|(k, v)| {
                let description = if describe {
                    joy_core::model::project::describe_value(&k, &v)
                } else {
                    None
                };
                Entry {
                    key: k,
                    value: value_as_optional_string(&v),
                    description,
                }
            })
            .collect();
        return crate::output::emit(Payload { key, entries });
    }

    if leaves.is_empty() {
        std::process::exit(1);
    }

    let rows: Vec<(String, String, Option<String>)> = leaves
        .iter()
        .map(|(k, v)| {
            let display = scalar_str(v);
            let desc = if describe {
                joy_core::model::project::describe_value(k, v)
            } else {
                None
            };
            (k.clone(), display, desc)
        })
        .collect();

    let max_key = rows.iter().map(|(k, _, _)| k.len()).max().unwrap_or(0);
    let max_val = rows.iter().map(|(_, v, _)| v.len()).max().unwrap_or(0);

    for (k, v, desc) in &rows {
        if let Some(d) = desc {
            println!(
                "{:<kw$}  {:<vw$}  {} {}",
                color::label(k),
                v,
                color::inactive("-"),
                color::inactive(d),
                kw = max_key,
                vw = max_val,
            );
        } else {
            println!("{:<kw$}  {}", color::label(k), v, kw = max_key);
        }
    }
    Ok(())
}

/// Render `joy project get` for a list-typed key. Text form is one
/// entry per line (exit 1 if the list is empty so tooling can detect
/// the unset state, mirroring scalar-key behaviour). JSON form is a
/// `{key, value}` payload where `value` is the array (or null when
/// empty / unset, matching the existing API contract for other
/// optional keys).
fn get_list_value(root: &std::path::Path, key: &str, describe: bool) -> Result<()> {
    let entries = version_files_get(root)?;

    if crate::output::is_json() {
        #[derive(serde::Serialize)]
        struct GetListPayload<'a> {
            key: &'a str,
            value: Option<Vec<String>>,
            #[serde(skip_serializing_if = "Option::is_none")]
            description: Option<String>,
        }
        let description = if describe {
            joy_core::model::project::describe_value(key, &serde_json::Value::Null)
        } else {
            None
        };
        let value = if entries.is_empty() {
            None
        } else {
            Some(entries)
        };
        return crate::output::emit(GetListPayload {
            key,
            value,
            description,
        });
    }

    if entries.is_empty() {
        std::process::exit(1);
    }

    let suffix = if describe {
        joy_core::model::project::describe_value(key, &serde_json::Value::Null)
            .map(|d| format!("  {} {}", color::inactive("-"), color::inactive(&d)))
            .unwrap_or_default()
    } else {
        String::new()
    };

    for (i, entry) in entries.iter().enumerate() {
        if i == 0 {
            println!("{entry}{suffix}");
        } else {
            println!("{entry}");
        }
    }
    Ok(())
}

/// Dispatch a `joy project set` invocation. Handles scalar keys via the
/// existing set_value() path and list keys (`release.version-files`)
/// via the dedicated version-files helpers that operate on raw YAML so
/// mapping-form entries round-trip cleanly.
fn set_command(ctx: &Context, project: &mut Project, args: SetArgs) -> Result<()> {
    let key = &args.key;

    if is_list_key(key) {
        return set_list_key(
            ctx,
            key,
            args.value.as_deref(),
            args.add.as_deref(),
            args.rm.as_deref(),
            args.editor.as_deref(),
        );
    }

    if args.add.is_some() || args.rm.is_some() {
        bail!(
            "'{key}' is not a list-typed key; --add and --rm only apply to: {}",
            LIST_KEYS.join(", ")
        );
    }

    if key == "privacy" {
        return set_privacy(ctx, project, &args);
    }

    let value = match args.value.as_deref() {
        Some(v) => v.to_string(),
        None => match editor_scalar_value(project, key, args.editor.as_deref())? {
            EditorOutcome::Apply(v) => v,
            EditorOutcome::NoOp => {
                println!("{key} unchanged");
                return Ok(());
            }
        },
    };

    set_value(project, key, &value)?;
    store::save_project(&ctx.root, project)?;
    if key == "acronym" {
        let stored = project.acronym.as_deref().unwrap_or(&value);
        println!("{key} = {stored}");
        println!();
        println!("Note: existing items keep their previous ID prefix.");
        println!("Only items created after this change use the new prefix '{stored}'.");
        println!();
        println!("Local delegation keys have been migrated to the new acronym.");
        println!("Existing sessions and delegation tokens reference the old acronym");
        println!("and are invalidated. Re-run `joy auth` and reissue any tokens.");
    } else {
        println!("{key} = {value}");
    }
    let log_user = ctx.log_user();
    joy_core::git_ops::auto_git_post_command(
        &ctx.root,
        &format!("project set {key} {value}"),
        &log_user,
    );
    Ok(())
}

/// Handle `joy project set privacy <none|open|anonymous>`. Switching to or from
/// `anonymous` is an atomic working-tree migration (ADR-042) that needs the
/// operator's unlocked seed; the manage capability is already enforced by the
/// caller. `open`/`none` on a project that is not anonymous is a plain field
/// normalization.
fn set_privacy(ctx: &Context, project: &mut Project, args: &SetArgs) -> Result<()> {
    let target = args.value.as_deref().map(str::trim).unwrap_or_default();
    let want_anon = match target {
        "anonymous" => true,
        "open" | "none" => false,
        other => bail!("invalid privacy mode '{other}'; expected: none, open, or anonymous"),
    };
    let is_anon = project.privacy_mode() == PrivacyMode::Anonymous;

    if want_anon && is_anon {
        println!("privacy already anonymous");
        return Ok(());
    }
    if !want_anon && !is_anon {
        // Plain field normalization, no migration.
        set_value(project, "privacy", target)?;
        store::save_project(&ctx.root, project)?;
        println!("privacy = {target}");
        return Ok(());
    }

    // A real switch: unlock the acting member's seed (auth), then migrate.
    let unlocked = crate::auth_gate::unlock_acting(&ctx.root, project)?;

    let renamed = if want_anon {
        joy_core::privacy::switch_to_anonymous(&ctx.root, project, &unlocked.seed)?
    } else {
        joy_core::privacy::switch_to_open(&ctx.root, project, &unlocked.seed)?
    };

    // The migration rekeys every human member, but git config (or the
    // forge account) still names the person by their real address, which
    // `member_key_for_email` resolves in either mode (open: the address
    // itself; anonymous: the opaque id whose `email_match` verifies
    // against it, ADR-042). With the device pin gone (JOY-02AE-1A) there
    // is no stale key to repair here any more: the next command re-reads
    // git config and finds the migrated key on its own.

    // The migration rewrote project.yaml, members.yaml, items and logs.
    joy_core::git_ops::auto_git_add(&ctx.root, &[store::JOY_DIR]);
    let log_user = ctx.log_user();
    joy_core::git_ops::auto_git_post_command(
        &ctx.root,
        &format!("project set privacy {target}"),
        &log_user,
    );
    let n = renamed.len();
    println!(
        "privacy = {} ({n} member{} migrated)",
        if want_anon { "anonymous" } else { "open" },
        if n == 1 { "" } else { "s" }
    );
    Ok(())
}

/// Apply a list-key mutation. Exactly one of `value` (CSV replace),
/// `add_path`, or `rm_path` carries the operation; clap's
/// `conflicts_with` enforces that the other two are absent.
fn set_list_key(
    ctx: &Context,
    key: &str,
    value: Option<&str>,
    add_path: Option<&str>,
    rm_path: Option<&str>,
    editor_flag: Option<&str>,
) -> Result<()> {
    assert!(is_list_key(key));

    // When no value and no flag, fall through to the editor.
    if value.is_none() && add_path.is_none() && rm_path.is_none() {
        return editor_list_value(ctx, key, editor_flag);
    }

    let (summary, display) = if let Some(path) = add_path {
        let outcome = version_files_add(&ctx.root, path)?;
        let summary = format!("project set {key} --add {path}");
        let display = match outcome {
            AddOutcome::Added => format!("{key} += {path}"),
            AddOutcome::AlreadyPresent => {
                println!("warning: '{path}' already configured in {key}; nothing to do");
                format!("{key} unchanged ({path} already present)")
            }
        };
        (summary, display)
    } else if let Some(path) = rm_path {
        version_files_rm(&ctx.root, path)?;
        let summary = format!("project set {key} --rm {path}");
        let display = format!("{key} -= {path}");
        (summary, display)
    } else {
        let raw = value
            .ok_or_else(|| anyhow::anyhow!("value required for '{key}' (or use --add / --rm)"))?;
        let paths: Vec<String> = if raw.trim().is_empty() {
            Vec::new()
        } else {
            raw.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };
        version_files_set(&ctx.root, &paths)?;
        let summary = format!("project set {key} {raw}");
        let display = if paths.is_empty() {
            format!("{key} = (cleared)")
        } else {
            format!("{key} = {}", paths.join(","))
        };
        (summary, display)
    };

    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    joy_core::git_ops::auto_git_add(&ctx.root, &[&rel]);
    println!("{display}");
    let log_user = ctx.log_user();
    joy_core::git_ops::auto_git_post_command(&ctx.root, &summary, &log_user);
    Ok(())
}

enum EditorOutcome {
    /// User saved a new value (possibly an empty string to clear).
    Apply(String),
    /// User saved the editor buffer unchanged.
    NoOp,
}

/// Open $EDITOR for a scalar key. Initial buffer is the current
/// value (or empty when unset). The user's saved buffer is taken
/// as-is (trimmed); per-key validation runs downstream when the
/// returned value is fed into set_value(). Returns NoOp when the
/// buffer comes back unchanged.
fn editor_scalar_value(
    project: &Project,
    key: &str,
    editor_flag: Option<&str>,
) -> Result<EditorOutcome> {
    let current = current_scalar_value(project, key);
    let initial = current.clone();
    let edited = crate::editor::edit_text(editor_flag, &initial, &editor_file_suffix(key))?;
    let new_value = edited.unwrap_or_default();
    if new_value.trim() == initial.trim() {
        return Ok(EditorOutcome::NoOp);
    }
    Ok(EditorOutcome::Apply(new_value))
}

/// Open $EDITOR for a list-typed key. Initial buffer is a short
/// `#`-prefixed header explaining the format, followed by the
/// current entries one per line. On save, `#`-comment lines and
/// blank lines are stripped; the remaining lines are the new list.
/// Same NoOp / clear / apply semantics as the scalar path; on Apply
/// the list goes through version_files_set() (no per-entry
/// validation today beyond non-empty).
fn editor_list_value(ctx: &Context, key: &str, editor_flag: Option<&str>) -> Result<()> {
    let current = version_files_get(&ctx.root)?;
    let initial = list_editor_buffer(key, &current);

    let edited = crate::editor::edit_text(editor_flag, &initial, &editor_file_suffix(key))?;
    let new_entries: Vec<String> = match edited {
        None => Vec::new(),
        Some(content) => content
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.to_string())
            .collect(),
    };

    if new_entries == current {
        println!("{key} unchanged");
        return Ok(());
    }

    version_files_set(&ctx.root, &new_entries)?;
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    joy_core::git_ops::auto_git_add(&ctx.root, &[&rel]);
    if new_entries.is_empty() {
        println!("{key} = (cleared)");
    } else {
        println!("{key} = {}", new_entries.join(","));
    }
    let log_user = ctx.log_user();
    joy_core::git_ops::auto_git_post_command(
        &ctx.root,
        &format!("project set {key} (via editor)"),
        &log_user,
    );
    Ok(())
}

/// Render the editor buffer for a list-typed key: a header with the
/// stripping rules, followed by one entry per line.
fn list_editor_buffer(key: &str, entries: &[String]) -> String {
    let mut buf = String::new();
    buf.push_str(&format!("# joy project set {key}\n"));
    buf.push_str("# One entry per line. Lines starting with # and blank lines are ignored.\n");
    buf.push_str("# Save an empty body (no entries) to clear the list.\n");
    for entry in entries {
        buf.push_str(entry);
        buf.push('\n');
    }
    buf
}

fn editor_file_suffix(key: &str) -> String {
    let normalized: String = key
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("project-{normalized}.txt")
}

fn show_project(project: &Project, root: &std::path::Path) {
    println!("{}", color::header(&project.name));

    let w = 14;
    if let Some(ref acronym) = project.acronym {
        println!("{}", color::key_value("Acronym:", acronym, w));
    }
    let description = project
        .description
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("(unset)");
    println!("{}", color::key_value("Description:", description, w));
    println!("{}", color::key_value("Language:", &project.language, w));
    // Privacy mode (ADR-042). Always shown so the active mode is visible at a
    // glance; `open` is the effective default when unset.
    println!(
        "{}",
        color::key_value("Privacy:", &project.privacy_mode().to_string(), w)
    );
    if let Some(forge) = project.forge.as_deref() {
        println!("{}", color::key_value("Forge:", forge, w));
    }
    println!(
        "{}",
        color::key_value(
            "Created:",
            &project.created.format("%Y-%m-%d %H:%M").to_string(),
            w
        )
    );

    // Docs paths. Always rendered with their effective (defaulted)
    // values so operators see at a glance which files the project is
    // wired to, matching what `joy project get docs.*` reports.
    println!("\n{}:", color::label("Docs"));
    let docs_w = 16;
    println!(
        "  {}",
        color::key_value(
            "Architecture:",
            project.docs.architecture_or_default(),
            docs_w
        )
    );
    println!(
        "  {}",
        color::key_value("Vision:", project.docs.vision_or_default(), docs_w)
    );
    println!(
        "  {}",
        color::key_value(
            "Contributing:",
            project.docs.contributing_or_default(),
            docs_w
        )
    );

    if project.has_members() {
        println!("\n{}:", color::label("Members"));
        print_members_table(project, root);
    }

    // Workflow visualization with gates
    show_workflow(root);

    println!("{}", color::label(&"-".repeat(color::terminal_width())));

    // Hint about member modes if AI members exist
    if project
        .member_keys()
        .any(|id| joy_core::model::project::is_ai_member(id))
    {
        println!(
            "{}",
            color::label("Use `joy project member show <ID>` to see interaction levels")
        );
    }
}

/// Register an AI member and issue its token: what `joy ai add` does
/// before it sets the tool up, and the same thing `joy project member
/// add <name> --with-token` does.
pub(crate) fn add_ai_member(
    name: &str,
    adapter: Option<String>,
    model: Option<String>,
) -> Result<()> {
    let mut ctx = Context::load()?;
    let mut project = store::load_project(&ctx.root)?;
    let add = MemberAddArgs {
        id: name.to_string(),
        capabilities: Vec::new(),
        level: None,
        adapter,
        model,
        description: None,
        with_token: true,
    };
    run_member(
        MemberArgs {
            command: Some(MemberCommand::Add(add)),
        },
        &mut project,
        &mut ctx,
    )
}

fn run_member(args: MemberArgs, project: &mut Project, ctx: &mut Context) -> Result<()> {
    match args.command {
        None => {
            if crate::output::is_json() {
                // The members map is keyed by the at-rest id; resolve the keys
                // for output so --json never exposes a raw opaque id, identical
                // to the terminal (ADR-042). Value fields resolve via their own
                // MemberRef serialization.
                let resolved: std::collections::BTreeMap<String, &Member> = project
                    .members()
                    .map(|(id, m)| (joy_core::member_ref::resolve_str(id), m))
                    .collect();
                return crate::output::emit(resolved);
            }
            // List members
            if !project.has_members() {
                println!("No members configured.");
            } else {
                print_members_table(project, &ctx.root);
            }
        }
        Some(MemberCommand::Show(a)) => {
            // `a.id` is a user-supplied identifier. It may be an at-rest map key
            // (an `ai:` id, or the opaque `m-...` id a user reads from
            // project.yaml in anonymous mode, or a cleartext e-mail in open mode
            // where the key *is* the e-mail) or a human e-mail in anonymous mode.
            // Try the key space first (preserves the original by-key lookup, incl.
            // the opaque-id case the no-raw-id test exercises), then fall back to
            // resolving an e-mail. (ADR-042)
            let member = project
                .member_by_key(&a.id)
                .or_else(|| project.member_by_email(&a.id))
                .ok_or_else(|| anyhow::anyhow!("member not found: {}", a.id))?;

            let key = project
                .member_key(&a.id)
                .or_else(|| project.member_key_for_email(&a.id))
                .unwrap_or_else(|| a.id.clone());
            let is_ai = joy_core::model::project::is_ai_member(&key);
            // What an AI member may do has three sides: what the project
            // allows, what the person looking allows it themselves, and
            // what comes out for them (JI-019D-46).
            let viewer = joy_core::identity::acting_human_key(&ctx.root).ok();
            let may = (is_ai && joy_core::auth::grants::applies(project))
                .then(|| joy_core::auth::grants::view(project, &key, viewer.as_deref()));

            if crate::output::is_json() {
                #[derive(serde::Serialize)]
                struct Side {
                    capabilities: Vec<Capability>,
                    level: joy_core::model::config::InteractionLevel,
                }
                #[derive(serde::Serialize)]
                struct May {
                    project: Option<Side>,
                    mine: Option<Side>,
                    effective: Option<Side>,
                }
                #[derive(serde::Serialize)]
                struct ShowPayload<'a> {
                    id: joy_core::member_ref::MemberRef,
                    member: &'a joy_core::model::project::Member,
                    #[serde(skip_serializing_if = "Option::is_none")]
                    may: Option<May>,
                }
                let side = |e: &joy_core::auth::grants::Effective| Side {
                    capabilities: e.capabilities.clone(),
                    level: e.level,
                };
                return crate::output::emit(ShowPayload {
                    id: key.clone().into(),
                    member,
                    may: may.as_ref().map(|v| May {
                        project: v.project.as_ref().ok().map(side),
                        mine: v.mine.as_ref().map(side),
                        effective: v.effective.as_ref().ok().map(side),
                    }),
                });
            }

            let w = color::terminal_width();
            println!(
                "{}",
                color::header(&joy_core::member_ref::resolve_str(&key))
            );

            match &may {
                Some(view) => {
                    let runs_on = [member.adapter.as_deref(), member.model.as_deref()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" · ");
                    if !runs_on.is_empty() {
                        println!("  {}", color::inactive(&runs_on));
                    }
                    if let Some(description) = &member.description {
                        println!("  {description}");
                    }
                    let project_side = view.project.as_ref().ok();
                    let mine = view.mine.as_ref();
                    let effective = view.effective.as_ref().ok();
                    println!(
                        "  {:<12} {:<12} {:<12} {}",
                        "",
                        color::label("project"),
                        color::label("mine"),
                        color::label("effective")
                    );
                    for cap in joy_core::model::item::Capability::ALL {
                        if !project_side.is_some_and(|p| p.allows(cap)) {
                            continue;
                        }
                        println!(
                            "  {:<12} {:<12} {:<12} {}",
                            cap.to_string(),
                            may_column(project_side, cap),
                            may_column(mine, cap),
                            may_column(effective, cap)
                        );
                    }
                    let level = |side: Option<&joy_core::auth::grants::Effective>| {
                        side.map(|s| s.level.to_string()).unwrap_or_default()
                    };
                    println!(
                        "  {:<12} {:<12} {:<12} {}",
                        "level",
                        level(project_side),
                        level(mine),
                        level(effective)
                    );
                    for why in [view.project.as_ref().err(), view.effective.as_ref().err()]
                        .into_iter()
                        .flatten()
                        .take(1)
                    {
                        println!("  {}", color::warning(why));
                    }
                }
                None => {
                    for cap in joy_core::model::item::Capability::ALL {
                        let mark = if member.has_capability(cap) { "x" } else { "-" };
                        println!("  {:<12} {}", cap.to_string(), mark);
                    }
                }
            }

            println!("{}", color::label(&"-".repeat(w)));
        }
        Some(MemberCommand::Add(a)) => {
            crate::auth_gate::enforce(ctx, &Action::ManageProject, "project")?;
            if project.has_member_key(&a.id) {
                bail!("member {} already exists", a.id);
            }
            // In anonymous mode a human member must be onboarded through the OTP
            // enrollment flow (opaque id + members.yaml entry + zone-key wrap),
            // not added by e-mail key, which would write cleartext PII into
            // project.yaml. Until that flow lands, refuse rather than leak; the
            // documented path is to add the member in open mode and switch back.
            // AI members carry no PII and keep their readable id, so they are fine.
            if project.privacy_mode() == PrivacyMode::Anonymous
                && !joy_core::model::project::is_ai_member(&a.id)
            {
                bail!(
                    "cannot add a human member while privacy is anonymous: it would write \
                     the e-mail in cleartext.\nAdd them in open mode and switch back:\n  \
                     joy project set privacy open\n  joy project member add {}\n  \
                     joy project set privacy anonymous",
                    a.id
                );
            }
            let is_ai = joy_core::model::project::is_ai_member(&a.id);
            if !is_ai
                && (a.level.is_some()
                    || a.adapter.is_some()
                    || a.model.is_some()
                    || a.description.is_some())
            {
                bail!("--level, --adapter, --model and --description are for an AI member");
            }
            let capabilities = match parse_capabilities(&a.capabilities)? {
                NamedCapabilities::All => MemberCapabilities::All,
                NamedCapabilities::List(list) => specific(&list),
                // What an AI member starts with is the project's to say
                // (`ai-defaults` in project.yaml), a person gets the same
                // set unless told otherwise.
                NamedCapabilities::Unsaid if is_ai => {
                    let defaults = joy_core::store::load_ai_defaults(&ctx.root).capabilities;
                    if defaults.is_empty() {
                        default_member_capabilities()
                    } else {
                        specific(&defaults)
                    }
                }
                NamedCapabilities::Unsaid => default_member_capabilities(),
            };
            let level = a.level.as_deref().map(parse_level).transpose()?;

            // Authenticate the acting manage member by passphrase. Their
            // identity key will sign the attestation placed on the new
            // member's entry (JOY-00FC-1D).
            let attester_key = joy_core::identity::acting_human_key(&ctx.root)?;

            // One unlock serves the attestation and, with `--with-token`,
            // the delegation token too (JOY-0185-66): the gate answers
            // from the session or asks once.
            let attester = crate::auth_gate::unlock(&ctx.root, project, &attester_key)?;
            let attester_kp = &attester.keypair;

            // AI members do not enrol via passphrase; they get a delegation
            // token issued by an existing operator (`joy auth token add`).
            // Skip the OTP machinery for them so the on-screen instructions
            // do not point at the wrong flow (JOY-016F-16).

            let (otp_opt, otp_hash_opt) = if is_ai {
                (None, None)
            } else {
                let otp = joy_core::auth::otp::generate_otp();
                let otp_hash = joy_core::auth::otp::hash_otp(&otp)?;
                (Some(otp), Some(otp_hash))
            };

            // The acting manager signs for the new entry (JOY-00FC-1D);
            // what exactly is signed is joy-core's to say.
            let mut new_member = Member::new(capabilities);
            new_member.enrollment_verifier = otp_hash_opt;
            if is_ai {
                joy_core::auth::grants::change_maximum(&mut new_member, None, level)?;
                // The tool that runs it: said, or the one its name names.
                let name = joy_core::model::project::ai_member_name(&a.id);
                new_member.adapter = match a.adapter.as_deref() {
                    Some(adapter) => Some(
                        joy_ai::naming::tool_adapter(adapter)
                            .ok_or_else(|| anyhow::anyhow!("unknown adapter: {adapter}"))?
                            .to_string(),
                    ),
                    None => joy_ai::naming::tool_adapter(name).map(String::from),
                };
                new_member.model = a.model.clone();
                new_member.description = a.description.clone();
            }
            joy_core::auth::vouch::sign(
                project,
                &attester_key,
                attester_kp,
                &a.id,
                &mut new_member,
                joy_core::auth::vouch::Occasion::New,
            );
            project.register_member(&a.id, new_member)?;

            store::save_project(&ctx.root, project)?;

            // Optional immediate token issuance for AI members
            // (JOY-0185-66): the same unlocked operator signs it, so
            // nothing is asked a second time.
            let token_result: Option<(String, i64)> = if is_ai && a.with_token {
                Some(crate::commands::auth::create_delegation_token(
                    &ctx.root, &attester, &a.id, None,
                )?)
            } else {
                None
            };

            if crate::output::is_json() {
                #[derive(serde::Serialize)]
                struct AddPayload<'a> {
                    member: &'a str,
                    otp: Option<&'a str>,
                    #[serde(skip_serializing_if = "Option::is_none")]
                    token: Option<&'a str>,
                    #[serde(skip_serializing_if = "Option::is_none")]
                    ttl_hours: Option<i64>,
                }
                crate::output::emit(AddPayload {
                    member: &a.id,
                    otp: otp_opt.as_deref(),
                    token: token_result.as_ref().map(|(t, _)| t.as_str()),
                    ttl_hours: token_result.as_ref().map(|(_, h)| *h),
                })?;
            } else {
                println!("Added member {}", color::user(&a.id));
                if let Some((ref token, _hours)) = token_result {
                    println!();
                    println!("\"{}\"", token);
                } else if is_ai {
                    println!();
                    println!("Next steps:");
                    println!("  1. Issue a delegation token:");
                    println!("       joy auth token add {}", a.id);
                    println!("  2. Share the token with the AI in chat.");
                    println!("  3. The AI redeems it with:");
                    println!("       joy auth --token <TOKEN> --json");
                    println!(
                        "     and picks up `member` as its identity and `session_env` as auth."
                    );
                    println!("  4. The AI reads `joy ai tutorial` for the operational guide.");
                    println!();
                    println!("Tip: rerun with `--with-token` to combine the two steps next time.");
                } else if let Some(ref otp) = otp_opt {
                    println!();
                    println!("  One-time password: {otp}");
                    println!();
                    println!(
                        "Share the OTP with {} via a trusted channel. They redeem it with:",
                        a.id
                    );
                    println!("  joy auth --otp {otp}");
                }
            }

            let log_user = ctx.log_user();
            joy_core::git_ops::auto_git_post_command(
                &ctx.root,
                &format!("project member add {}", a.id),
                &log_user,
            );
        }
        Some(MemberCommand::Edit(a)) => {
            let named = parse_capabilities(&a.capabilities)?;
            let level = a.level.as_deref().map(parse_level).transpose()?;
            let nothing_said = matches!(named, NamedCapabilities::Unsaid)
                && a.add_capability.is_empty()
                && a.rm_capability.is_empty()
                && level.is_none()
                && a.model.is_none()
                && a.description.is_none();
            if nothing_said {
                bail!(
                    "nothing to edit: pass --capabilities, --add-capability, \
                     --rm-capability, --level, --model or --description"
                );
            }

            // The member as the project knows it: an AI member's name in
            // either spelling, an opaque id, or an address resolved
            // through the privacy layer (ADR-042).
            let key = project
                .member_key(&a.id)
                .or_else(|| joy_core::privacy::member_key_for_email(project, &a.id))
                .ok_or_else(|| anyhow::anyhow!("member not found: {}", a.id))?;
            let is_ai = joy_core::model::project::is_ai_member(&key);
            if !is_ai
                && (level.is_some() || a.project || a.model.is_some() || a.description.is_some())
            {
                bail!("--level, --project, --model and --description are for an AI member");
            }
            let parse_all = |words: &[String]| -> Result<Vec<Capability>> {
                words
                    .iter()
                    .map(|w| {
                        w.trim()
                            .parse()
                            .map_err(|e: String| anyhow::anyhow!("{}", e))
                    })
                    .collect()
            };
            let (adding, removing) = (parse_all(&a.add_capability)?, parse_all(&a.rm_capability)?);

            if is_ai && !a.project {
                // What I allow this AI member myself, within what the
                // project allows it: signed with my own key, and nobody
                // needs to hold manage for it (JI-019D-46).
                if a.model.is_some() || a.description.is_some() {
                    bail!("--model and --description change the project's side: add --project");
                }
                let me = joy_core::identity::acting_human_key(&ctx.root)?;
                let unlocked = crate::auth_gate::unlock(&ctx.root, project, &me)?;
                let view = joy_core::auth::grants::view(project, &key, Some(&me));
                let allowed = view.project.clone().map_err(|why| anyhow::anyhow!(why))?;
                let mut caps: Option<Vec<Capability>> = match named {
                    NamedCapabilities::Unsaid => None,
                    NamedCapabilities::All => Some(allowed.capabilities.clone()),
                    NamedCapabilities::List(list) => Some(list),
                };
                if !adding.is_empty() || !removing.is_empty() {
                    let mut list = caps
                        .or_else(|| view.mine.map(|m| m.capabilities))
                        .unwrap_or(allowed.capabilities);
                    list.retain(|cap| !removing.contains(cap));
                    for cap in adding {
                        if !list.contains(&cap) {
                            list.push(cap);
                        }
                    }
                    caps = Some(list);
                }
                joy_core::auth::grants::set_personal(
                    project,
                    &key,
                    &me,
                    &unlocked.keypair,
                    caps.as_deref(),
                    level,
                )?;
            } else {
                crate::auth_gate::enforce(ctx, &Action::ManageProject, "project")?;
                let mut member = project
                    .member_by_key(&key)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("member not found: {}", a.id))?;
                let had_manage = member.has_capability(&Capability::Manage);

                match named {
                    NamedCapabilities::Unsaid => {}
                    NamedCapabilities::All if is_ai => {
                        bail!("an AI member never holds the manage capability: name what it may do")
                    }
                    NamedCapabilities::All => member.set_capabilities(MemberCapabilities::All),
                    NamedCapabilities::List(list) => member.set_capabilities(specific(&list)),
                }
                if !adding.is_empty() || !removing.is_empty() {
                    match &mut member.capabilities {
                        MemberCapabilities::All if !removing.is_empty() => bail!(
                            "member has 'capabilities: all'; replace the set with \
                             --capabilities <list> before removing individual capabilities"
                        ),
                        MemberCapabilities::All => {}
                        MemberCapabilities::Specific(map) => {
                            for cap in &adding {
                                map.entry(*cap).or_default();
                            }
                            for cap in &removing {
                                map.remove(cap);
                            }
                        }
                    }
                }
                if is_ai {
                    if member.has_capability(&Capability::Manage) {
                        bail!("an AI member never holds the manage capability");
                    }
                    joy_core::auth::grants::change_maximum(&mut member, None, level)?;
                    if let Some(model) = &a.model {
                        member.model = (!model.is_empty()).then(|| model.clone());
                    }
                    if let Some(description) = &a.description {
                        member.description = (!description.is_empty()).then(|| description.clone());
                    }
                }

                // Anti-brick: never strip manage from the last manager.
                if had_manage && !member.has_capability(&Capability::Manage) {
                    let guard = joy_core::guard::Guard::new(project);
                    if guard.is_last_manager(&key) {
                        bail!(
                            "cannot remove manage from {}: last member with manage \
                             capability. Grant another member manage first.",
                            a.id
                        );
                    }
                }

                // The acting manager signs what changed, where the
                // project keeps a signature for it (joy-core says where).
                let acting_key = joy_core::identity::acting_human_key(&ctx.root)?;
                let acting_kp = derive_acting_keypair(&ctx.root, project, &acting_key)?;
                joy_core::auth::vouch::sign(
                    project,
                    &acting_key,
                    &acting_kp,
                    &key,
                    &mut member,
                    joy_core::auth::vouch::Occasion::Changed,
                );
                *project
                    .member_by_key_mut(&key)
                    .expect("member key resolved above") = member;
            }
            store::save_project(&ctx.root, project)?;
            // The tool a member is named after is set up with the member's
            // level: bring its files along, so the change holds the next
            // time the tool starts and not only after `joy update`.
            if is_ai && a.project {
                let tool = joy_core::model::project::ai_member_name(&key);
                if joy_ai::ai_setup::is_tool_configured(&ctx.root, tool) {
                    joy_ai::ai_setup::configure_tool(&ctx.root, tool, &mut |_| {})?;
                }
            }

            if crate::output::is_json() {
                #[derive(serde::Serialize)]
                struct EditPayload<'a> {
                    member: &'a str,
                }
                crate::output::emit(EditPayload { member: &key })?;
            } else {
                println!("Updated member {}", color::user(&a.id));
            }
            let log_user = ctx.log_user();
            joy_core::git_ops::auto_git_post_command(
                &ctx.root,
                &format!("project member edit {}", a.id),
                &log_user,
            );
        }
        Some(MemberCommand::Rm(a)) => {
            crate::auth_gate::enforce(ctx, &Action::ManageProject, "project")?;

            // JOY-00FE-F6: self-remove is blocked and directs the user to
            // another manage member.
            let acting_key = joy_core::identity::acting_human_key(&ctx.root)?;
            if a.id == acting_key {
                let others: Vec<&String> = project
                    .members()
                    .filter(|(key, m)| **key != acting_key && m.has_capability(&Capability::Manage))
                    .map(|(email, _)| email)
                    .collect();
                let list = if others.is_empty() {
                    "(no other manage members; add one first via `joy project member add <email>`)"
                        .to_string()
                } else {
                    others
                        .iter()
                        .map(|e| format!("  - {e}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                bail!(
                    "Cannot remove yourself. Another manage member must perform this action.\n\
                     Current manage members:\n{list}"
                );
            }

            // Prevent removing the last member with manage capability.
            let guard = joy_core::guard::Guard::new(project);
            if guard.is_last_manager(&a.id) {
                bail!(
                    "cannot remove {}: last member with manage capability. \
                     Add another manage-capable member first.",
                    a.id
                );
            }

            // JOY-00FF-93: collect members whose attester is the one being
            // removed; they need to be re-attested by the acting manage
            // member so the attestation chain stays intact.
            let removed_id = a.id.clone();
            let orphans = joy_core::auth::vouch::signed_by(project, &removed_id);

            let acting_kp = if orphans.is_empty() {
                None
            } else {
                Some(derive_acting_keypair(&ctx.root, project, &acting_key)?)
            };

            if project.remove_member(&a.id).is_none() {
                bail!("member not found: {}", a.id);
            }

            // Re-attest all orphans with the acting member's key. Capabilities
            // and otp_hash of each orphan are preserved (they don't change).
            if let Some(kp) = acting_kp {
                for orphan_email in &orphans {
                    let orphan = project
                        .member_by_key(orphan_email)
                        .cloned()
                        .expect("orphan exists - just collected");
                    let mut orphan = orphan;
                    joy_core::auth::vouch::sign(
                        project,
                        &acting_key,
                        &kp,
                        orphan_email,
                        &mut orphan,
                        joy_core::auth::vouch::Occasion::SignerLeft,
                    );
                    *project.member_by_key_mut(orphan_email).unwrap() = orphan;
                }
            }

            store::save_project(&ctx.root, project)?;
            if crate::output::is_json() {
                #[derive(serde::Serialize)]
                struct RmPayload<'a> {
                    removed_member: &'a str,
                }
                crate::output::emit(RmPayload {
                    removed_member: &a.id,
                })?;
            } else {
                println!("Removed member {}", color::user(&a.id));
            }
            let log_user = ctx.log_user();
            joy_core::git_ops::auto_git_post_command(
                &ctx.root,
                &format!("project member rm {}", a.id),
                &log_user,
            );
        }
        Some(MemberCommand::Erase(a)) => {
            crate::auth_gate::enforce(ctx, &Action::ManageProject, "project")?;
            if project.privacy_mode() != PrivacyMode::Anonymous {
                bail!("erasure applies only to anonymous projects (privacy: anonymous)");
            }
            // Unlock the acting manage member's seed; it grants members.yaml access.
            let operator_key = joy_core::identity::acting_human_key(&ctx.root)?;
            let unlocked = crate::auth_gate::unlock(&ctx.root, project, &operator_key)?;

            // The target is an opaque id already in members.yaml, or an e-mail
            // resolved to its id via the email_match verifier.
            let target_id = if project.has_member_key(&a.id) {
                a.id.clone()
            } else {
                joy_core::privacy::member_key_for_email(project, &a.id)
                    .ok_or_else(|| anyhow::anyhow!("no member matches {}", a.id))?
            };
            let removed =
                joy_core::privacy::erase_member(&ctx.root, project, &unlocked.seed, &target_id)?;
            let rel = format!(
                "{}/{}",
                store::JOY_DIR,
                joy_core::members_file::MEMBERS_FILE
            );
            joy_core::git_ops::auto_git_add(&ctx.root, &[&rel]);
            if removed {
                println!(
                    "Erased {target_id} from members.yaml. The opaque id, verifier and audit \
                     trail remain; no Joy output can resolve it to a person anymore."
                );
            } else {
                println!(
                    "No members.yaml entry for {}; nothing to erase.",
                    color::user(&a.id)
                );
            }
            let log_user = ctx.log_user();
            joy_core::git_ops::auto_git_post_command(
                &ctx.root,
                &format!("project member erase {target_id}"),
                &log_user,
            );
        }
    }
    Ok(())
}

/// Default capability set for newly added members. Excludes `manage` and
/// `delete`: those must be granted explicitly via `--capabilities`, so a
/// forgotten flag cannot silently hand over project administration or
/// destructive rights (principle of least privilege).
fn default_member_capabilities() -> MemberCapabilities {
    let mut map = std::collections::BTreeMap::new();
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
    MemberCapabilities::Specific(map)
}

/// The acting member's identity keypair, through the auth gate: the
/// session of this terminal, else the passphrase (which then makes the
/// session). Used to sign attestations on `joy project member add`.
///
/// `member_key` is an at-rest member map key, the shape
/// [`joy_core::identity::acting_human_key`] answers with, so an anonymous
/// project (ADR-042) needs no second lookup path.
pub(crate) fn derive_acting_keypair(
    root: &std::path::Path,
    project: &Project,
    member_key: &str,
) -> Result<IdentityKeypair> {
    Ok(crate::auth_gate::unlock(root, project, member_key)?.keypair)
}

fn print_members_table(project: &Project, root: &std::path::Path) {
    use joy_core::model::item::Capability;

    let cap_headers: &[(&str, Capability)] = &[
        ("con", Capability::Conceive),
        ("pln", Capability::Plan),
        ("des", Capability::Design),
        ("imp", Capability::Implement),
        ("tst", Capability::Test),
        ("rev", Capability::Review),
        ("doc", Capability::Document),
        ("crt", Capability::Create),
        ("asg", Capability::Assign),
        ("mng", Capability::Manage),
        ("del", Capability::Delete),
    ];

    let use_emoji = color::use_emoji();

    // Resolve auth status for each member
    let project_id = joy_core::auth::session::project_id(root).unwrap_or_default();
    let auth_statuses: Vec<(&str, String)> = project
        .members()
        .map(|(id, member)| {
            let auth = member_auth_status(id, member, project, &project_id, use_emoji);
            (id.as_str(), auth)
        })
        .collect();

    let w_auth = auth_statuses
        .iter()
        .map(|(_, a)| display_width(a))
        .max()
        .unwrap_or(4)
        .max(4);

    // Resolve each member id to its display value (ADR-042): name/e-mail in
    // anonymous mode, the key itself in open mode. Column width is sized on the
    // resolved value so the table never lays out around a raw opaque id.
    let display_names: Vec<String> = project
        .member_keys()
        .map(|id| joy_core::member_ref::resolve_str(id))
        .collect();
    let max_member = display_names
        .iter()
        .map(|n| n.len())
        .max()
        .unwrap_or(6)
        .max(6);
    let term_width = color::terminal_width();

    // chmod-style capability string: cpditrw/camd (12 chars) or "all" (3 chars)
    // Work: conceive plan design implement test review write(doc)
    // Mgmt: create assign manage delete
    let chmod_width = 12; // "cpditrw/camd"

    // Fixed columns: "  " prefix + " " auth gap + " " caps gap
    let fixed = 2 + 1 + w_auth + 1;

    // Try wide mode (x-matrix): needs 4 chars per cap column
    let caps_wide = cap_headers.len() * 4;
    let w_member_wide = term_width.saturating_sub(fixed + caps_wide);

    // Compact mode (chmod-style): needs 12 chars for caps
    let w_member_compact = term_width.saturating_sub(fixed + chmod_width);

    let (w_member, wide_mode) = if w_member_wide >= 12 {
        (w_member_wide.min(max_member), true)
    } else {
        (w_member_compact.min(max_member).max(8), false)
    };

    // Header
    print!(
        "  {}",
        color::inactive(&format!("{:<w$}", "Member", w = w_member))
    );
    print!(" {}", color::inactive(&pad_right("Auth", w_auth)));
    if wide_mode {
        for (hdr, _) in cap_headers {
            print!(" {}", color::inactive(&format!("{:<3}", hdr)));
        }
    } else {
        // chmod-style header
        print!(" {}", color::inactive("Caps"));
    }
    println!();

    // Rows
    for (((_id, member), (_, auth)), display_name) in project
        .members()
        .zip(auth_statuses.iter())
        .zip(display_names.iter())
    {
        let display_id = truncate(display_name, w_member);
        print!("  {:<w$}", display_id, w = w_member);
        print!(" {}", pad_right(auth, w_auth));

        if wide_mode {
            for (_, cap) in cap_headers {
                let has = match &member.capabilities {
                    MemberCapabilities::All => true,
                    MemberCapabilities::Specific(map) => map.contains_key(cap),
                };
                if has {
                    if cap.is_management() {
                        print!("  {} ", color::warning("x"));
                    } else {
                        print!("  x ");
                    }
                } else {
                    print!("    ");
                }
            }
        } else {
            // chmod-style: cpditrw/camd
            print!(" {}", caps_chmod(member, cap_headers));
        }
        println!();
    }
}

/// Render capabilities in chmod-style: `cpditrw/camd`
/// Work caps: conceive(c) plan(p) design(d) implement(i) test(t) review(r) write/doc(w)
/// Mgmt caps: create(c) assign(a) manage(m) delete(d)
/// Missing caps shown as `-`. `all` renders as colored "all".
fn caps_chmod(
    member: &Member,
    _cap_headers: &[(&str, joy_core::model::item::Capability)],
) -> String {
    use joy_core::model::item::Capability;

    if member.capabilities == MemberCapabilities::All {
        return color::warning("all");
    }

    // Single-char labels for each capability in order
    let chars: &[(char, &Capability)] = &[
        ('c', &Capability::Conceive),
        ('p', &Capability::Plan),
        ('d', &Capability::Design),
        ('i', &Capability::Implement),
        ('t', &Capability::Test),
        ('r', &Capability::Review),
        ('w', &Capability::Document),
    ];
    let mgmt_chars: &[(char, &Capability)] = &[
        ('c', &Capability::Create),
        ('a', &Capability::Assign),
        ('m', &Capability::Manage),
        ('d', &Capability::Delete),
    ];

    let has = |cap: &Capability| -> bool {
        match &member.capabilities {
            MemberCapabilities::All => true,
            MemberCapabilities::Specific(map) => map.contains_key(cap),
        }
    };

    let work: String = chars
        .iter()
        .map(|(ch, cap)| if has(cap) { *ch } else { '-' })
        .collect();

    let mgmt: String = mgmt_chars
        .iter()
        .map(|(ch, cap)| if has(cap) { *ch } else { '-' })
        .collect();

    // Color the management part if any management caps are present
    let has_mgmt = mgmt.chars().any(|c| c != '-');
    if has_mgmt {
        format!("{}/{}", work, color::warning(&mgmt))
    } else {
        format!("{}/----", work)
    }
}

/// Show the workflow visualization with gate markers.
fn show_workflow(root: &std::path::Path) {
    let guard = joy_core::guard::Guard::load(root).ok();
    let empty_gates = std::collections::BTreeMap::new();
    let gates = guard.as_ref().map(|g| g.gates()).unwrap_or(&empty_gates);
    let use_emoji = color::use_emoji();

    println!("\n{}:", color::label("Workflow"));

    // Gate marker for a transition
    let gate_marker = |from: &str, to: &str| -> bool {
        let key = format!("{from} -> {to}");
        gates.get(&key).map(|g| !g.allow_ai).unwrap_or(false)
    };

    let gated_arrow = |from: &str, to: &str| -> String {
        if gate_marker(from, to) {
            if use_emoji {
                "─⛔─>".to_string()
            } else {
                color::warning("-X->")
            }
        } else {
            "──>".to_string()
        }
    };

    let term_width = color::terminal_width();

    if term_width >= 72 {
        // Wide: horizontal flow
        let a1 = gated_arrow("new", "open");
        let a2 = gated_arrow("open", "in-progress");
        let a3 = gated_arrow("in-progress", "review");
        let a4 = gated_arrow("review", "closed");

        println!(
            "  new {} open {} in-progress {} review {} closed",
            a1, a2, a3, a4
        );
        println!("   │                                  │");
        println!("   └──> deferred <────────────────────┘");
    } else {
        // Narrow: vertical
        let arr = |from: &str, to: &str| -> String {
            if gate_marker(from, to) {
                if use_emoji {
                    "⛔".to_string()
                } else {
                    color::warning("X")
                }
            } else {
                "│".to_string()
            }
        };
        println!("  new");
        println!("  {} open", arr("new", "open"));
        println!("  │   {} in-progress", arr("open", "in-progress"));
        println!("  │   │   {} review", arr("in-progress", "review"));
        println!("  │   │   │   {} closed", arr("review", "closed"));
        println!("  │   └──> deferred");
        println!("  └──> deferred");
    }

    // Gate list
    if gates.is_empty() {
        println!("\n  {}", color::inactive("Gates: none configured"));
    } else {
        println!("\n  {}:", color::label("Gates"));
        for (key, gate) in gates {
            let mut rules = Vec::new();
            if !gate.allow_ai {
                rules.push("allow_ai: false");
            }
            if !rules.is_empty() {
                println!(
                    "    {} {:<24} {}",
                    color::warn_mark(),
                    color::warning(key),
                    rules.join(", ")
                );
            }
        }
    }
}

/// Display width of a string (accounts for Unicode and ANSI escapes).
fn display_width(s: &str) -> usize {
    // Strip ANSI escape codes before measuring
    let stripped = s
        .replace("\x1b[33m", "")
        .replace("\x1b[0m", "")
        .replace("\x1b[38;5;208m", "");
    unicode_width::UnicodeWidthStr::width(stripped.as_str())
}

/// Pad a string to a target display width with spaces.
fn pad_right(s: &str, target: usize) -> String {
    let w = display_width(s);
    if w >= target {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(target - w))
    }
}

/// Truncate a string to max width, adding `…` if shortened.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else if max <= 1 {
        "…".to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}

/// Determine auth status string for a member.
fn member_auth_status(
    id: &str,
    member: &Member,
    all_members: &Project,
    project_id: &str,
    use_emoji: bool,
) -> String {
    use joy_core::model::project::is_ai_member;

    let is_ai = is_ai_member(id);

    // For humans: has passphrase key?
    // For AI: a human registered an ai_delegations entry, which is the
    // one and only channel now (JI-0174 family).
    let has_delegation = is_ai
        && all_members
            .member_values()
            .any(|m| m.delegation_to(id).is_some());
    let has_auth = if is_ai {
        has_delegation
    } else {
        member.verify_key.is_some()
    };

    // Session check: must mirror what resolve_identity (joy-core) actually
    // accepts at runtime. A check mark that the runtime would reject is
    // exactly the divergence JOY-00F4-CF closes -- the display and the
    // auth behaviour must agree.
    let has_session = if !has_auth {
        false
    } else if is_ai {
        // AI sessions (ADR-033): a session file alone is not enough; the
        // caller must hold the matching ephemeral private key in
        // JOY_SESSION. Otherwise sessions are "present on disk but not
        // usable from this shell". The env sid names the session file
        // directly (one file per session, JOY-01E1-E7), so the check is a
        // straight lookup of the env-referenced session.
        let current_delegation_keys: Vec<&str> = all_members
            .member_values()
            .filter_map(|m| m.delegation_to(id))
            .map(|entry| entry.delegation_verifier.as_str())
            .collect();

        // Drop this member's dead sessions: expired ones, and ones bound
        // to a rotated (or missing) delegation key.
        if let Ok(sessions) = joy_core::auth::session::list_member_sessions(project_id, id) {
            for (path, sess) in &sessions {
                let rotated = !matches!(
                    &sess.claims.delegation_key,
                    Some(tk) if current_delegation_keys.contains(&tk.as_str())
                );
                if sess.claims.expires <= chrono::Utc::now() || rotated {
                    if let Some(sid) = path.file_stem().and_then(|s| s.to_str()) {
                        let _ = joy_core::auth::session::remove_session_by_id(sid);
                    }
                }
            }
        }

        std::env::var("JOY_SESSION")
            .ok()
            .and_then(|v| joy_core::auth::session::parse_session_env(&v))
            .and_then(|(sid, _)| {
                joy_core::auth::session::load_session_by_id(&sid)
                    .ok()
                    .flatten()
            })
            .and_then(|sess| {
                if sess.claims.expires <= chrono::Utc::now()
                    || sess.claims.member != id
                    || sess.claims.project_id != project_id
                {
                    return None;
                }
                // Mirrors what resolve_identity accepts at runtime
                // (JOY-00F4-CF): a session is live while the delegation
                // it was redeemed from is live.
                match &sess.claims.delegation_key {
                    Some(tk) if current_delegation_keys.contains(&tk.as_str()) => Some(()),
                    // Delegation rotated — the session is no longer trusted.
                    _ => None,
                }?;
                Some(())
            })
            .is_some()
    } else if let Some(pk_hex) = member.verify_key.as_ref() {
        if let Ok(pk) = joy_core::auth::PublicKey::from_hex(pk_hex) {
            joy_core::auth::session::load_session(project_id, id)
                .ok()
                .flatten()
                .and_then(|token| {
                    let claims = joy_core::auth::session::validate_session(&token, &pk, project_id)
                        .ok()
                        .filter(|c| c.member == id)?;
                    // Human sessions are TTY-bound (see resolve_identity in
                    // joy-core). A session created in TTY-A must not be
                    // reported as active in TTY-B.
                    if claims.tty != joy_core::auth::session::current_tty() {
                        return None;
                    }
                    Some(())
                })
                .is_some()
        } else {
            false
        }
    } else {
        false
    };

    if use_emoji {
        if !has_auth {
            "· ·".to_string()
        } else if is_ai {
            if has_session {
                "✓ 🎟️".to_string()
            } else {
                "· 🎟️".to_string()
            }
        } else if has_session {
            "✓ 🔐".to_string()
        } else {
            "· 🔐".to_string()
        }
    } else if !has_auth {
        "--".to_string()
    } else {
        // `tok` = delegation-token channel, `key` = human passphrase key.
        let kind = if is_ai { "tok" } else { "key" };
        if has_session {
            color::warning(&format!("{kind}+s"))
        } else {
            color::warning(kind)
        }
    }
}

fn complete_project_key(
    current: &std::ffi::OsStr,
) -> Vec<clap_complete::engine::CompletionCandidate> {
    let Some(prefix) = current.to_str() else {
        return Vec::new();
    };
    PROJECT_KEYS
        .iter()
        .filter(|k| k.starts_with(prefix))
        .map(|k| clap_complete::engine::CompletionCandidate::new(*k))
        .collect()
}
