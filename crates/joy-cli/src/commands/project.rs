// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::{bail, Result};
use clap::Args;

use joy_core::auth::IdentityKeypair;
use joy_core::context::Context;
use joy_core::guard::Action;
use joy_core::model::item::Capability;
use joy_core::model::project::{
    validate_acronym, CapabilityConfig, Member, MemberCapabilities, PrivacyMode,
};
use joy_core::model::Project;
use joy_core::store;
use joy_core::vcs::Vcs;

use crate::color;

const PROJECT_KEYS: &[&str] = &[
    "name",
    "acronym",
    "description",
    "language",
    "forge",
    "privacy",
    "created",
    "docs.architecture",
    "docs.vision",
    "docs.contributing",
    "release.version-files",
];

/// Keys whose value is a list rather than a scalar. List keys accept
/// `--add` / `--rm` flags on `joy project set` plus CSV form for whole-
/// list replacement; their `get` output is one entry per line (or a
/// JSON array under --json). Scalar keys reject `--add`/`--rm`.
const LIST_KEYS: &[&str] = &["release.version-files"];

/// Parse an interaction-level argument value; an empty string means "clear"
/// (`None`), anything else must be one of the three level names.
fn parse_optional_level(s: &str) -> Result<Option<joy_core::model::config::InteractionLevel>> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    s.parse::<joy_core::model::config::InteractionLevel>()
        .map(Some)
        .map_err(|e| anyhow::anyhow!("{}", e))
}

fn is_list_key(key: &str) -> bool {
    LIST_KEYS.contains(&key)
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
    /// Passphrase for a `privacy` mode switch (non-interactive). Falls back to
    /// JOY_PASSPHRASE or an interactive prompt.
    #[arg(long)]
    passphrase: Option<String>,
    /// Read the passphrase from a single line on stdin (for `privacy` switch).
    #[arg(long = "passphrase-stdin")]
    passphrase_stdin: bool,
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
    /// Edit a member's capabilities and interaction levels
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

    /// Passphrase of the acting manage member (non-interactive).
    #[arg(long)]
    passphrase: Option<String>,

    /// Read the passphrase from a single line on stdin.
    #[arg(long = "passphrase-stdin")]
    passphrase_stdin: bool,
}

#[derive(clap::Args)]
struct MemberShowArgs {
    /// Member ID (email or ai:tool@joy)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_member))]
    id: String,
}

#[derive(clap::Args)]
struct MemberAddArgs {
    /// Member ID (email or ai:tool@joy)
    id: String,

    /// Capabilities: comma-separated list, or the keyword `all`.
    /// Default: conceive, plan, design, implement, test, review,
    /// document, create, assign. `manage` and `delete` must be
    /// granted explicitly (use `all` to include them).
    #[arg(short = 'c', long)]
    capabilities: Option<String>,

    /// Passphrase of the acting manage member (non-interactive, for
    /// scripts and tests). The acting member's identity key signs the
    /// attestation placed on the new member's entry.
    #[arg(long)]
    passphrase: Option<String>,

    /// Read the passphrase from a single line on stdin.
    #[arg(long = "passphrase-stdin")]
    passphrase_stdin: bool,

    /// After registering an AI member, immediately issue a delegation
    /// token. Combines `joy project member add` and `joy auth token add`
    /// so the operator unlocks their identity once. Ignored for human
    /// members.
    #[arg(long = "with-token")]
    with_token: bool,
}

#[derive(clap::Args)]
struct MemberRmArgs {
    /// Member ID (email or ai:tool@joy)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_member))]
    id: String,

    /// Passphrase of the acting manage member (non-interactive, for
    /// scripts and tests). Required when the removed member attested
    /// others, so re-attestation can be signed by the remover.
    #[arg(long)]
    passphrase: Option<String>,

    /// Read the passphrase from a single line on stdin.
    #[arg(long = "passphrase-stdin")]
    passphrase_stdin: bool,
}

#[derive(clap::Args)]
struct MemberEditArgs {
    /// Member ID (email or ai:tool@joy)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_member))]
    id: String,

    /// Replace the whole capability set: a comma-separated list, or the
    /// keyword `all`. Surviving capabilities keep their
    /// interaction-level/max-interaction-level/max-cost settings.
    #[arg(short = 'c', long, conflicts_with_all = ["add_capability", "rm_capability"])]
    capabilities: Option<String>,

    /// Grant one capability, keeping the rest (repeatable).
    #[arg(long = "add-capability", value_name = "CAP")]
    add_capability: Vec<String>,

    /// Revoke one capability, keeping the rest (repeatable).
    #[arg(long = "rm-capability", value_name = "CAP")]
    rm_capability: Vec<String>,

    /// Set the member default interaction level: `LEVEL` for the global
    /// default, `CAP=LEVEL` per capability (repeatable); `=` resp. `CAP=`
    /// clears. LEVEL is one of proposing|confirmed|autonomous.
    #[arg(long = "interaction-level", value_name = "[CAP=]LEVEL")]
    interaction_level: Vec<String>,

    /// Set a per-capability max-interaction-level floor: `CAP=LEVEL`
    /// (repeatable); `CAP=` clears it. LEVEL is one of
    /// proposing|confirmed|autonomous.
    #[arg(long = "max-interaction-level", value_name = "CAP=LEVEL")]
    max_interaction_level: Vec<String>,

    /// Passphrase of the acting manage member (non-interactive, for
    /// scripts and tests). Any capability or interaction change invalidates the
    /// member's attestation, so the acting member re-signs it.
    #[arg(long)]
    passphrase: Option<String>,

    /// Read the passphrase from a single line on stdin.
    #[arg(long = "passphrase-stdin")]
    passphrase_stdin: bool,
}

pub fn run(args: ProjectArgs) -> Result<()> {
    let ctx = Context::load()?;

    let project_path = store::joy_dir(&ctx.root).join(store::PROJECT_FILE);
    let mut project: Project = store::read_yaml(&project_path)?;

    match args.command {
        Some(ProjectCommand::Get(a)) => {
            return get_value(&ctx.root, &project, &a.key, a.describe);
        }
        Some(ProjectCommand::Set(a)) => {
            ctx.enforce(&Action::ManageProject, "project")?;
            return set_command(&ctx, &project_path, &mut project, a);
        }
        Some(ProjectCommand::Member(a)) => {
            return run_member(a, &mut project, &project_path, &ctx);
        }
        None => {}
    }

    // Legacy flag-based editing
    let is_edit = args.name.is_some() || args.description.is_some() || args.language.is_some();

    if is_edit {
        ctx.enforce(&Action::ManageProject, "project")?;
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
        store::write_yaml_preserve(&project_path, &project)?;
        let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
        joy_core::git_ops::auto_git_add(&ctx.root, &[&rel]);
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

/// Strip a trailing `.*` (or bare `*`) from `key` and return the
/// prefix that remains. `None` when the key has no wildcard.
fn wildcard_prefix(key: &str) -> Option<&str> {
    if key == "*" {
        Some("")
    } else {
        key.strip_suffix(".*")
    }
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

fn scalar_str(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Some project fields are `Option<String>` (acronym, description).
/// In the existing JSON contract those return `null` when unset, not
/// the string `"null"`. Preserve that.
fn value_as_optional_string(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// Snapshot the project's read-exposed metadata into a nested JSON
/// tree so `flatten_under` and `describe_value` can walk it the same
/// way `joy config get` does. The shape matches PROJECT_KEYS: top-level
/// scalars plus a `docs` object holding the three resolved doc paths.
/// Unset optional fields (acronym, description) are represented as
/// `null` so the existing JSON payload shape on those keys is
/// preserved.
fn project_value_tree(root: &std::path::Path, project: &Project) -> serde_json::Value {
    let version_files: serde_json::Value = match version_files_get(root) {
        Ok(v) if !v.is_empty() => {
            serde_json::Value::Array(v.into_iter().map(serde_json::Value::String).collect())
        }
        _ => serde_json::Value::Null,
    };
    serde_json::json!({
        "name": project.name,
        "acronym": project.acronym,
        "description": project.description,
        "language": project.language,
        "forge": project.forge,
        "privacy": project.privacy().map(|p| p.to_string()).unwrap_or_else(|| "none".to_string()),
        "created": project.created.format("%Y-%m-%d %H:%M").to_string(),
        "docs": {
            "architecture": project.docs.architecture_or_default(),
            "vision": project.docs.vision_or_default(),
            "contributing": project.docs.contributing_or_default(),
        },
        "release": {
            "version-files": version_files,
        }
    })
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
fn set_command(
    ctx: &Context,
    project_path: &std::path::Path,
    project: &mut Project,
    args: SetArgs,
) -> Result<()> {
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
        return set_privacy(ctx, project_path, project, &args);
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
    store::write_yaml_preserve(project_path, project)?;
    if key.starts_with("docs.") {
        prune_docs_yaml(project_path, &project.docs)?;
    }
    if key == "forge" && project.forge.is_none() {
        prune_yaml_key(project_path, "forge")?;
    }
    if key == "privacy" && project.privacy().is_none() {
        prune_yaml_key(project_path, "privacy")?;
    }
    if key == "description" && project.description.is_none() {
        prune_yaml_key(project_path, "description")?;
    }
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    joy_core::git_ops::auto_git_add(&ctx.root, &[&rel]);
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
fn set_privacy(
    ctx: &Context,
    project_path: &std::path::Path,
    project: &mut Project,
    args: &SetArgs,
) -> Result<()> {
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
        store::write_yaml_preserve(project_path, project)?;
        if project.privacy().is_none() {
            prune_yaml_key(project_path, "privacy")?;
        }
        let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
        joy_core::git_ops::auto_git_add(&ctx.root, &[&rel]);
        println!("privacy = {target}");
        return Ok(());
    }

    // A real switch: unlock the acting member's seed (auth), then migrate.
    let git_email = joy_core::vcs::default_vcs().user_email()?;
    let member_key = joy_core::privacy::member_key_for_email(project, &git_email)
        .ok_or_else(|| anyhow::anyhow!("{git_email} is not a member of this project"))?;
    let member = project
        .member_by_key(&member_key)
        .expect("member_key came from the member map");
    if member.verify_key.is_none() {
        bail!("{git_email} has no identity. Run `joy auth init` first.");
    }
    let passphrase = match args.passphrase.clone().or_else(|| {
        std::env::var("JOY_PASSPHRASE")
            .ok()
            .filter(|s| !s.is_empty())
    }) {
        Some(p) => p,
        None => {
            crate::commands::auth::read_passphrase(None, args.passphrase_stdin, "Passphrase: ")?
        }
    };
    let unlocked = joy_core::auth::unlock_identity(member, &passphrase)?;

    let renamed = if want_anon {
        joy_core::privacy::switch_to_anonymous(&ctx.root, project, &unlocked.seed)?
    } else {
        joy_core::privacy::switch_to_open(&ctx.root, project, &unlocked.seed)?
    };

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

enum AddOutcome {
    Added,
    AlreadyPresent,
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

/// Read the current scalar value for `key` as plain text. Returns
/// empty string for unset Option fields and for `docs.*` overrides
/// at the built-in default (callers can tell the user this is the
/// default by inspecting the value, but for editor pre-population
/// the empty case is fine).
fn current_scalar_value(project: &Project, key: &str) -> String {
    match key {
        "name" => project.name.clone(),
        "acronym" => project.acronym.clone().unwrap_or_default(),
        "description" => project.description.clone().unwrap_or_default(),
        "language" => project.language.clone(),
        "forge" => project.forge.clone().unwrap_or_default(),
        "privacy" => project
            .privacy()
            .map(|p| p.to_string())
            .unwrap_or_else(|| "none".to_string()),
        "docs.architecture" => project.docs.architecture.clone().unwrap_or_default(),
        "docs.vision" => project.docs.vision.clone().unwrap_or_default(),
        "docs.contributing" => project.docs.contributing.clone().unwrap_or_default(),
        _ => String::new(),
    }
}

fn editor_file_suffix(key: &str) -> String {
    let normalized: String = key
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("project-{normalized}.txt")
}

/// Extract the path field from a release.version-files entry. Each
/// entry is either a bare string or a mapping with a `path` field
/// (with optional extra fields preserved on round-trip).
fn entry_path(entry: &serde_yaml_ng::Value) -> Option<String> {
    use serde_yaml_ng::Value;
    match entry {
        Value::String(s) => Some(s.clone()),
        Value::Mapping(m) => m
            .get(Value::String("path".into()))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

/// Load raw release.version-files entries from project.yaml,
/// preserving mapping-form entries verbatim.
fn version_files_raw(root: &std::path::Path) -> Result<Vec<serde_yaml_ng::Value>> {
    let path = store::joy_dir(root).join(store::PROJECT_FILE);
    let raw = std::fs::read_to_string(&path)?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&raw)?;
    let Some(map) = doc.as_mapping() else {
        return Ok(Vec::new());
    };
    let Some(release) = map.get(serde_yaml_ng::Value::String("release".into())) else {
        return Ok(Vec::new());
    };
    let Some(release_map) = release.as_mapping() else {
        return Ok(Vec::new());
    };
    let Some(files) = release_map.get(serde_yaml_ng::Value::String("version-files".into())) else {
        return Ok(Vec::new());
    };
    let Some(seq) = files.as_sequence() else {
        bail!("release.version-files in project.yaml is not a list");
    };
    Ok(seq.clone())
}

/// Read the configured paths as plain strings (mapping-form entries
/// are reduced to their `path` field).
fn version_files_get(root: &std::path::Path) -> Result<Vec<String>> {
    Ok(version_files_raw(root)?
        .into_iter()
        .filter_map(|e| entry_path(&e))
        .collect())
}

fn version_files_add(root: &std::path::Path, path: &str) -> Result<AddOutcome> {
    let mut entries = version_files_raw(root)?;
    if entries
        .iter()
        .any(|e| entry_path(e).as_deref() == Some(path))
    {
        return Ok(AddOutcome::AlreadyPresent);
    }
    entries.push(serde_yaml_ng::Value::String(path.to_string()));
    write_version_files_raw(root, entries)?;
    Ok(AddOutcome::Added)
}

fn version_files_rm(root: &std::path::Path, path: &str) -> Result<()> {
    let mut entries = version_files_raw(root)?;
    let before = entries.len();
    entries.retain(|e| entry_path(e).as_deref() != Some(path));
    if entries.len() == before {
        bail!("'{path}' is not configured in release.version-files");
    }
    write_version_files_raw(root, entries)?;
    Ok(())
}

fn version_files_set(root: &std::path::Path, paths: &[String]) -> Result<()> {
    let entries = paths
        .iter()
        .map(|p| serde_yaml_ng::Value::String(p.clone()))
        .collect();
    write_version_files_raw(root, entries)
}

/// Write the supplied entries back to release.version-files,
/// creating the `release:` block if needed and removing
/// `version-files` (or the entire `release:` block if it becomes
/// empty) when entries is empty.
fn write_version_files_raw(
    root: &std::path::Path,
    entries: Vec<serde_yaml_ng::Value>,
) -> Result<()> {
    use serde_yaml_ng::Value;
    let path = store::joy_dir(root).join(store::PROJECT_FILE);
    let raw = std::fs::read_to_string(&path)?;
    let mut doc: Value = serde_yaml_ng::from_str(&raw)?;
    let Some(top) = doc.as_mapping_mut() else {
        bail!("project.yaml is not a mapping");
    };
    let release_key = Value::String("release".into());
    let version_key = Value::String("version-files".into());

    if entries.is_empty() {
        // Remove version-files; drop the release block too if it becomes empty.
        if let Some(release) = top.get_mut(&release_key) {
            if let Some(release_map) = release.as_mapping_mut() {
                release_map.remove(&version_key);
                if release_map.is_empty() {
                    top.remove(&release_key);
                }
            }
        }
    } else {
        let release = top
            .entry(release_key)
            .or_insert_with(|| Value::Mapping(Default::default()));
        let Some(release_map) = release.as_mapping_mut() else {
            bail!("project.yaml release: is not a mapping");
        };
        release_map.insert(version_key, Value::Sequence(entries));
    }

    let yaml = serde_yaml_ng::to_string(&doc)?;
    std::fs::write(&path, yaml)?;
    Ok(())
}

fn set_value(project: &mut Project, key: &str, value: &str) -> Result<()> {
    match key {
        "name" => project.name = value.to_string(),
        "description" => {
            project.description = if value.is_empty() || value == "none" {
                None
            } else {
                Some(value.to_string())
            };
        }
        "language" => project.language = value.to_string(),
        "forge" => project.forge = normalize_forge_value(value)?,
        "privacy" => match value.trim() {
            "none" => project.set_privacy_non_anonymous(None)?,
            "open" => project.set_privacy_non_anonymous(Some(PrivacyMode::Open))?,
            "anonymous" => anyhow::bail!(
                "privacy: anonymous is not yet implemented; it arrives with the mode-transition task JOY-01BF-2E"
            ),
            other => {
                anyhow::bail!("invalid privacy mode '{other}'; expected: none, open, or anonymous")
            }
        },
        "docs.architecture" => project.docs.architecture = normalize_docs_value(value),
        "docs.vision" => project.docs.vision = normalize_docs_value(value),
        "docs.contributing" => project.docs.contributing = normalize_docs_value(value),
        "acronym" => {
            let normalized = validate_acronym(value).map_err(|e| anyhow::anyhow!(e))?;
            project.acronym = Some(normalized);
        }
        "created" => {
            anyhow::bail!("'created' is read-only");
        }
        _ => anyhow::bail!(
            "unknown key: {key}\nknown keys: {}",
            PROJECT_KEYS.join(", ")
        ),
    }
    Ok(())
}

/// Validate and normalize a `forge:` value. Empty input clears the
/// field (auto-detection at publish time applies). `"none"` is an
/// explicit opt-out and is stored verbatim so the intent is visible
/// in project.yaml. Any other value must be in
/// [`crate::forge::SUPPORTED_FORGES`]; this rejects typos at write
/// time, which is the right moment for strictness (read-time stays
/// lenient so legacy values don't hard-fail publish).
fn normalize_forge_value(value: &str) -> Result<Option<String>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed == "none" {
        return Ok(Some("none".to_string()));
    }
    if crate::forge::SUPPORTED_FORGES.contains(&trimmed) {
        return Ok(Some(trimmed.to_string()));
    }
    bail!(
        "unsupported forge '{trimmed}'\n  = help: supported values are: {}, none (pass an empty value to clear)",
        crate::forge::SUPPORTED_FORGES.join(", ")
    );
}

/// Empty / "none" / "default" reset a docs path to its built-in default.
fn normalize_docs_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("none")
        || trimmed.eq_ignore_ascii_case("default")
    {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Remove a top-level key from the on-disk project YAML. Used after
/// `write_yaml_preserve` to clear optional Option<String> fields that
/// the preserve step would otherwise re-add from the original file.
fn prune_yaml_key(path: &std::path::Path, key: &str) -> Result<()> {
    use serde_yaml_ng::Value;
    let raw = std::fs::read_to_string(path)?;
    let mut value: Value = serde_yaml_ng::from_str(&raw)?;
    if let Some(map) = value.as_mapping_mut() {
        map.remove(Value::String(key.to_string()));
    }
    let yaml = serde_yaml_ng::to_string(&value)?;
    std::fs::write(path, yaml)?;
    Ok(())
}

/// Rewrite the project YAML so the `docs:` block exactly reflects the desired
/// state. Removes the block entirely when no overrides are set; otherwise
/// replaces it with only the configured fields. Needed because
/// `write_yaml_preserve` keeps unknown top-level keys (which would otherwise
/// re-introduce a stale `docs:` block when an override is cleared).
fn prune_docs_yaml(path: &std::path::Path, docs: &joy_core::model::Docs) -> Result<()> {
    use serde_yaml_ng::Value;

    let raw = std::fs::read_to_string(path)?;
    let mut value: Value = serde_yaml_ng::from_str(&raw)?;
    let map = match value.as_mapping_mut() {
        Some(m) => m,
        None => return Ok(()),
    };
    let docs_key = Value::String("docs".to_string());
    if docs.is_empty() {
        map.remove(&docs_key);
    } else {
        let docs_value = serde_yaml_ng::to_value(docs)?;
        map.insert(docs_key, docs_value);
    }
    let yaml = serde_yaml_ng::to_string(&value)?;
    std::fs::write(path, yaml)?;
    Ok(())
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
    if project.member_keys().any(|id| id.starts_with("ai:")) {
        println!(
            "{}",
            color::label("Use `joy project member show <ID>` to see interaction levels")
        );
    }
}

fn run_member(
    args: MemberArgs,
    project: &mut Project,
    project_path: &std::path::Path,
    ctx: &Context,
) -> Result<()> {
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

            if crate::output::is_json() {
                #[derive(serde::Serialize)]
                struct ShowPayload<'a> {
                    id: joy_core::member_ref::MemberRef,
                    member: &'a joy_core::model::project::Member,
                }
                return crate::output::emit(ShowPayload {
                    id: a.id.clone().into(),
                    member,
                });
            }

            let w = color::terminal_width();
            let wide = w >= 60;

            println!(
                "{}",
                color::header(&joy_core::member_ref::resolve_str(&a.id))
            );

            // Load defaults for interaction-level resolution
            let raw_defaults = joy_core::store::load_raw_interaction_level_defaults(&ctx.root);
            let effective_defaults = joy_core::store::load_interaction_level_defaults(&ctx.root);
            let config = joy_core::store::load_config();
            let personal_level = if config.interaction_level.default
                != joy_core::model::config::InteractionLevel::default()
            {
                Some(config.interaction_level.default)
            } else {
                None
            };

            // Build capability list with has/denied and interaction info
            let all_caps = joy_core::model::item::Capability::ALL;
            let is_all = matches!(&member.capabilities, MemberCapabilities::All);
            let specific_map = match &member.capabilities {
                MemberCapabilities::Specific(map) => Some(map),
                _ => None,
            };

            for cap in all_caps {
                let has = is_all || specific_map.is_some_and(|m| m.contains_key(cap));
                let mark = if has { "x" } else { "-" };
                let cap_label = if wide {
                    format!("{cap}")
                } else {
                    let s = format!("{cap}");
                    s[..3].to_string()
                };

                if has && cap.is_work_capability() {
                    let cap_config = specific_map.and_then(|m| m.get(cap));
                    let (level, source) = joy_core::model::project::resolve_interaction_level(
                        cap,
                        &raw_defaults,
                        &effective_defaults,
                        member.interaction_level,
                        personal_level,
                        cap_config,
                    );
                    let level_text = format!("{level} [{source}]");
                    let mut line = if wide {
                        format!(
                            "  {:<12} {}   {}",
                            cap_label,
                            mark,
                            color::inactive(&level_text)
                        )
                    } else {
                        format!(
                            "  {:<5} {}   {}",
                            cap_label,
                            mark,
                            color::inactive(&level_text)
                        )
                    };
                    // Show the clamped-away preference if the floor won
                    if source == joy_core::model::project::InteractionLevelSource::ProjectMax {
                        if let Some(personal) = personal_level {
                            line.push_str(&color::inactive(&format!(
                                "  (your preference: {personal})"
                            )));
                        }
                    }
                    // Show max-interaction-level from cap config
                    if let Some(cc) = cap_config {
                        if let Some(ref max) = cc.max_interaction_level {
                            if source
                                != joy_core::model::project::InteractionLevelSource::ProjectMax
                            {
                                line.push_str(&color::inactive(&format!("  max: {max}")));
                            }
                        }
                    }
                    println!("{line}");
                } else if wide {
                    println!("  {:<12} {}", cap_label, mark);
                } else {
                    println!("  {:<5} {}", cap_label, mark);
                }
            }

            println!("{}", color::label(&"-".repeat(w)));
        }
        Some(MemberCommand::Add(a)) => {
            ctx.enforce(&Action::ManageProject, "project")?;
            if project.has_member_key(&a.id) {
                bail!("member {} already exists", a.id);
            }
            // In anonymous mode a human member must be onboarded through the OTP
            // enrollment flow (opaque id + members.yaml entry + zone-key wrap),
            // not added by e-mail key, which would write cleartext PII into
            // project.yaml. Until that flow lands, refuse rather than leak; the
            // documented path is to add the member in open mode and switch back.
            // AI members carry no PII and keep their readable id, so they are fine.
            if project.privacy_mode() == PrivacyMode::Anonymous && !a.id.starts_with("ai:") {
                bail!(
                    "cannot add a human member while privacy is anonymous: it would write \
                     the e-mail in cleartext.\nAdd them in open mode and switch back:\n  \
                     joy project set privacy open\n  joy project member add {}\n  \
                     joy project set privacy anonymous",
                    a.id
                );
            }
            let capabilities = match a.capabilities {
                None => default_member_capabilities(),
                Some(ref caps_str) if caps_str.trim() == "all" => MemberCapabilities::All,
                Some(ref caps_str) => {
                    let mut map = std::collections::BTreeMap::new();
                    for s in caps_str.split(',') {
                        let cap: Capability = s
                            .trim()
                            .parse()
                            .map_err(|e: String| anyhow::anyhow!("{}", e))?;
                        map.insert(cap, CapabilityConfig::default());
                    }
                    MemberCapabilities::Specific(map)
                }
            };

            // Authenticate the acting manage member by passphrase. Their
            // identity key will sign the attestation placed on the new
            // member's entry (JOY-00FC-1D).
            let attester_email = joy_core::vcs::default_vcs().user_email()?;
            let is_ai = a.id.starts_with("ai:");

            // When `--with-token` is set for an AI member, the same
            // passphrase signs the attestation *and* the delegation
            // token. Read it once here so the operator is prompted a
            // single time across both steps (JOY-0185-66).
            let captured_passphrase: Option<String> = if is_ai && a.with_token {
                Some(match a.passphrase.clone() {
                    Some(p) => p,
                    None => crate::commands::auth::read_passphrase(
                        None,
                        a.passphrase_stdin,
                        "Passphrase: ",
                    )?,
                })
            } else {
                a.passphrase.clone()
            };
            let attester_kp = derive_acting_keypair(
                project,
                &attester_email,
                captured_passphrase.as_deref(),
                a.passphrase_stdin,
            )?;

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

            // Construct and sign the attestation over (email, capabilities,
            // otp_hash). public_key is intentionally not covered.
            let signed_fields = joy_core::auth::attestation::signed_fields_for(
                &a.id,
                &capabilities,
                otp_hash_opt.as_deref(),
            );
            // Reference the attester by their on-disk member key so anonymous
            // mode (ADR-042) records the opaque id, never the cleartext e-mail.
            let attester_id = joy_core::privacy::member_key_for_email(project, &attester_email)
                .unwrap_or_else(|| attester_email.clone());
            let attestation = joy_core::auth::attestation::sign_attestation(
                &attester_id,
                &attester_kp,
                signed_fields,
            );

            let mut new_member = Member::new(capabilities);
            new_member.enrollment_verifier = otp_hash_opt;
            new_member.attestation = Some(attestation);
            project.register_member(&a.id, new_member)?;

            store::write_yaml_preserve(project_path, project)?;
            let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
            joy_core::git_ops::auto_git_add(&ctx.root, &[&rel]);

            // Optional immediate token issuance for AI members
            // (JOY-0185-66). Reuses the captured passphrase so the
            // operator is not prompted a second time.
            let token_result: Option<(String, i64)> = if is_ai && a.with_token {
                let passphrase = captured_passphrase
                    .as_deref()
                    .expect("captured when is_ai && with_token");
                Some(crate::commands::auth::create_delegation_token(
                    &ctx.root,
                    &attester_email,
                    passphrase,
                    &a.id,
                    None,
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
            ctx.enforce(&Action::ManageProject, "project")?;

            if a.capabilities.is_none()
                && a.add_capability.is_empty()
                && a.rm_capability.is_empty()
                && a.interaction_level.is_empty()
                && a.max_interaction_level.is_empty()
            {
                bail!(
                    "nothing to edit: pass --capabilities, --add-capability, \
                     --rm-capability, --interaction-level, or --max-interaction-level"
                );
            }

            // Resolve the target to its at-rest map key: an ai:/opaque id is
            // used as-is, a cleartext e-mail resolves via the privacy layer
            // (ADR-042) so anonymous mode never needs the e-mail here.
            let key = if project.has_member_key(&a.id) {
                a.id.clone()
            } else if let Some(k) = joy_core::privacy::member_key_for_email(project, &a.id) {
                k
            } else {
                bail!("member not found: {}", a.id);
            };
            let mut member = project
                .member_by_key(&key)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("member not found: {}", a.id))?;
            let had_manage = member.has_capability(&Capability::Manage);

            // 1. Capability-set changes. --capabilities replaces wholesale
            //    (carrying over surviving configs); --add/--rm-capability are
            //    incremental and mutually exclusive with it (clap-enforced).
            if let Some(caps_str) = a.capabilities.as_deref() {
                let target = if caps_str.trim() == "all" {
                    MemberCapabilities::All
                } else {
                    let mut map = std::collections::BTreeMap::new();
                    for s in caps_str.split(',') {
                        let cap: Capability = s
                            .trim()
                            .parse()
                            .map_err(|e: String| anyhow::anyhow!("{}", e))?;
                        map.insert(cap, CapabilityConfig::default());
                    }
                    MemberCapabilities::Specific(map)
                };
                member.set_capabilities(target);
            }
            for s in &a.add_capability {
                let cap: Capability = s
                    .trim()
                    .parse()
                    .map_err(|e: String| anyhow::anyhow!("{}", e))?;
                match &mut member.capabilities {
                    MemberCapabilities::All => {}
                    MemberCapabilities::Specific(map) => {
                        map.entry(cap).or_default();
                    }
                }
            }
            for s in &a.rm_capability {
                let cap: Capability = s
                    .trim()
                    .parse()
                    .map_err(|e: String| anyhow::anyhow!("{}", e))?;
                match &mut member.capabilities {
                    MemberCapabilities::All => bail!(
                        "member has 'capabilities: all'; replace the set with \
                         --capabilities <list> before removing individual capabilities"
                    ),
                    MemberCapabilities::Specific(map) => {
                        map.remove(&cap);
                    }
                }
            }

            // 2. Member default interaction levels: `LEVEL` sets the member's
            //    global default, `CAP=LEVEL` the per-capability default; an
            //    empty level clears the respective setting.
            for spec in &a.interaction_level {
                match spec.split_once('=') {
                    Some((cap_str, level_str)) => {
                        let cap: Capability = cap_str
                            .trim()
                            .parse()
                            .map_err(|e: String| anyhow::anyhow!("{}", e))?;
                        let level = parse_optional_level(level_str)?;
                        member
                            .set_capability_interaction_level(cap, level)
                            .map_err(|e| anyhow::anyhow!("{}", e))?;
                    }
                    None => {
                        member.interaction_level = parse_optional_level(spec)?;
                    }
                }
            }

            // 3. Per-capability max-interaction-level floors (CAP=LEVEL, CAP= clears).
            for spec in &a.max_interaction_level {
                let (cap_str, level_str) = spec.split_once('=').ok_or_else(|| {
                    anyhow::anyhow!(
                        "--max-interaction-level expects CAP=LEVEL (or CAP= to clear), got '{spec}'"
                    )
                })?;
                let cap: Capability = cap_str
                    .trim()
                    .parse()
                    .map_err(|e: String| anyhow::anyhow!("{}", e))?;
                let max_interaction_level = parse_optional_level(level_str)?;
                member
                    .set_capability_max_interaction_level(cap, max_interaction_level)
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
            }

            // 4. Anti-brick: never strip manage from the last manager.
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

            // 5. Re-sign: any capability or interaction-level change invalidates
            //    the stored attestation (it covers `capabilities`), so the
            //    acting manage member re-signs over the new fields.
            let acting_email = joy_core::vcs::default_vcs().user_email()?;
            let acting_kp = derive_acting_keypair(
                project,
                &acting_email,
                a.passphrase.as_deref(),
                a.passphrase_stdin,
            )?;
            let signed_fields = joy_core::auth::attestation::signed_fields_for(
                &key,
                &member.capabilities,
                member.enrollment_verifier.as_deref(),
            );
            let attester_id = joy_core::privacy::member_key_for_email(project, &acting_email)
                .unwrap_or_else(|| acting_email.clone());
            member.attestation = Some(joy_core::auth::attestation::sign_attestation(
                &attester_id,
                &acting_kp,
                signed_fields,
            ));

            // 5. Apply + persist + audit (mirrors add/rm).
            *project
                .member_by_key_mut(&key)
                .expect("member key resolved above") = member;
            store::write_yaml_preserve(project_path, project)?;
            let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
            joy_core::git_ops::auto_git_add(&ctx.root, &[&rel]);

            if crate::output::is_json() {
                #[derive(serde::Serialize)]
                struct EditPayload<'a> {
                    member: &'a str,
                }
                crate::output::emit(EditPayload { member: &a.id })?;
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
            ctx.enforce(&Action::ManageProject, "project")?;

            // JOY-00FE-F6: self-remove is blocked and directs the user to
            // another manage member.
            let acting_email = joy_core::vcs::default_vcs().user_email()?;
            if a.id == acting_email {
                let others: Vec<&String> = project
                    .members()
                    .filter(|(email, m)| {
                        **email != acting_email && m.has_capability(&Capability::Manage)
                    })
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
            let orphans: Vec<String> = project
                .members()
                .filter(|(email, m)| {
                    **email != removed_id
                        && m.attestation
                            .as_ref()
                            .map(|att| att.attester == removed_id.as_str())
                            .unwrap_or(false)
                })
                .map(|(email, _)| email.clone())
                .collect();

            let acting_kp = if orphans.is_empty() {
                None
            } else {
                Some(derive_acting_keypair(
                    project,
                    &acting_email,
                    a.passphrase.as_deref(),
                    a.passphrase_stdin,
                )?)
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
                    let signed_fields = joy_core::auth::attestation::signed_fields_for(
                        orphan_email,
                        &orphan.capabilities,
                        orphan.enrollment_verifier.as_deref(),
                    );
                    let new_attestation = joy_core::auth::attestation::sign_attestation(
                        &acting_email,
                        &kp,
                        signed_fields,
                    );
                    project.member_by_key_mut(orphan_email).unwrap().attestation =
                        Some(new_attestation);
                }
            }

            store::write_yaml_preserve(project_path, project)?;
            let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
            joy_core::git_ops::auto_git_add(&ctx.root, &[&rel]);
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
            ctx.enforce(&Action::ManageProject, "project")?;
            if project.privacy_mode() != PrivacyMode::Anonymous {
                bail!("erasure applies only to anonymous projects (privacy: anonymous)");
            }
            // Unlock the acting manage member's seed; it grants members.yaml access.
            let git_email = joy_core::vcs::default_vcs().user_email()?;
            let operator_key = joy_core::privacy::member_key_for_email(project, &git_email)
                .ok_or_else(|| anyhow::anyhow!("{git_email} is not a member of this project"))?;
            let operator = project
                .member_by_key(&operator_key)
                .expect("operator_key came from the member map");
            let passphrase = a.passphrase.clone().or_else(|| {
                std::env::var("JOY_PASSPHRASE")
                    .ok()
                    .filter(|s| !s.is_empty())
            });
            let passphrase = crate::commands::auth::read_passphrase(
                passphrase.as_deref(),
                a.passphrase_stdin,
                "Passphrase: ",
            )?;
            let unlocked = joy_core::auth::unlock_identity(operator, &passphrase)?;

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

/// Derive and verify the acting human member's identity keypair from their
/// passphrase. Used to sign attestations on `joy project member add`.
pub(crate) fn derive_acting_keypair(
    project: &Project,
    email: &str,
    passphrase_flag: Option<&str>,
    passphrase_stdin: bool,
) -> Result<IdentityKeypair> {
    let member = {
        // Resolve the member honoring the privacy mode: in anonymous mode
        // (ADR-042) the member map is keyed by an opaque id, not the cleartext
        // e-mail, so a direct `members.get(email)` would miss the founder when
        // `joy ai init` registers an AI member in an anonymous project.
        let member_key = joy_core::privacy::member_key_for_email(project, email)
            .ok_or_else(|| anyhow::anyhow!("{} is not a registered project member", email))?;
        project
            .member_by_key(&member_key)
            .expect("member_key resolved from email must exist")
    };
    if member.verify_key.is_none() {
        anyhow::bail!(
            "{} has no registered public key. Run `joy auth init` first.",
            email
        );
    }
    let passphrase =
        crate::commands::auth::read_passphrase(passphrase_flag, passphrase_stdin, "Passphrase: ")?;
    let unlocked = joy_core::auth::unlock_identity(member, &passphrase)?;
    Ok(unlocked.keypair)
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
            .any(|m| m.ai_delegations.contains_key(id));
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
            .filter_map(|m| m.ai_delegations.get(id))
            .map(|entry| entry.delegation_verifier.as_str())
            .collect();

        // Drop this member's dead sessions: expired ones, and
        // token-redeemed ones bound to a rotated delegation key. Rejected
        // job-bound sessions are kept on disk: their binding is enforced
        // at command time, not by file presence (JOY-020B-D2).
        if let Ok(sessions) = joy_core::auth::session::list_member_sessions(project_id, id) {
            for (path, sess) in &sessions {
                let rotated = sess.claims.job_id.is_none()
                    && !matches!(
                        &sess.claims.token_key,
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
                match &sess.claims.token_key {
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
