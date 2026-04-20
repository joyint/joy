// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::{bail, Result};
use clap::Args;

use joy_core::auth::{derive, sign};
use joy_core::context::Context;
use joy_core::guard::Action;
use joy_core::model::item::Capability;
use joy_core::model::project::{validate_acronym, CapabilityConfig, Member, MemberCapabilities};
use joy_core::model::Project;
use joy_core::store;
use joy_core::vcs::Vcs;

use crate::color;

const PROJECT_KEYS: &[&str] = &[
    "name",
    "acronym",
    "description",
    "language",
    "created",
    "docs.architecture",
    "docs.vision",
    "docs.contributing",
];

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
    /// Get a project value by key (name, acronym, description, language, created)
    Get(GetArgs),
    /// Set a project value by key (name, acronym, description, language)
    Set(SetArgs),
    /// Manage project members
    Member(MemberArgs),
}

#[derive(clap::Args)]
struct GetArgs {
    /// Project key
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(complete_project_key))]
    key: String,
}

#[derive(clap::Args)]
struct SetArgs {
    /// Project key
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(complete_project_key))]
    key: String,
    /// Value to set
    value: String,
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
    /// Remove a project member
    Rm(MemberRmArgs),
}

#[derive(clap::Args)]
struct MemberShowArgs {
    /// Member ID (email or ai:tool@joy)
    id: String,
}

#[derive(clap::Args)]
struct MemberAddArgs {
    /// Member ID (email or ai:tool@joy)
    id: String,

    /// Capabilities (comma-separated, default: all)
    #[arg(short = 'c', long)]
    capabilities: Option<String>,

    /// Passphrase of the acting manage member (non-interactive, for
    /// scripts and tests). The acting member's identity key signs the
    /// attestation placed on the new member's entry.
    #[arg(long)]
    passphrase: Option<String>,
}

#[derive(clap::Args)]
struct MemberRmArgs {
    /// Member ID (email or ai:tool@joy)
    id: String,
}

pub fn run(args: ProjectArgs) -> Result<()> {
    let ctx = Context::load()?;

    let project_path = store::joy_dir(&ctx.root).join(store::PROJECT_FILE);
    let mut project: Project = store::read_yaml(&project_path)?;

    match args.command {
        Some(ProjectCommand::Get(a)) => {
            return get_value(&project, &a.key);
        }
        Some(ProjectCommand::Set(a)) => {
            ctx.enforce(&Action::ManageProject, "project")?;
            // Acronym is embedded in the on-disk path of local delegation
            // keys. Capture the current project id before mutating so that,
            // after set_value flips the acronym, we can migrate the
            // delegations directory atomically before touching project.yaml.
            let old_project_id = joy_core::auth::session::project_id_of(&project);
            set_value(&mut project, &a.key, &a.value)?;
            if a.key == "acronym" {
                let new_project_id = joy_core::auth::session::project_id_of(&project);
                joy_core::auth::delegation::rename_project_delegations(
                    &old_project_id,
                    &new_project_id,
                )?;
            }
            store::write_yaml_preserve(&project_path, &project)?;
            // write_yaml_preserve re-adds top-level keys present in the
            // existing file but missing from the serialized struct (so unknown
            // keys are kept). When a `docs.*` key is unset the docs block in
            // the struct may serialize to nothing, but a stale `docs:` block
            // from the old file would be preserved -- prune it explicitly.
            if a.key.starts_with("docs.") {
                prune_docs_yaml(&project_path, &project.docs)?;
            }
            let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
            joy_core::git_ops::auto_git_add(&ctx.root, &[&rel]);
            if a.key == "acronym" {
                let stored = project.acronym.as_deref().unwrap_or(&a.value);
                println!("{} = {}", a.key, stored);
                println!();
                println!("Note: existing items keep their previous ID prefix.");
                println!("Only items created after this change use the new prefix '{stored}'.");
                println!();
                println!("Local delegation keys have been migrated to the new acronym.");
                println!("Existing sessions and delegation tokens reference the old acronym");
                println!("and are invalidated. Re-run `joy auth` and reissue any tokens.");
            } else {
                println!("{} = {}", a.key, a.value);
            }
            let log_user = ctx.log_user();
            joy_core::git_ops::auto_git_post_command(
                &ctx.root,
                &format!("project set {} {}", a.key, a.value),
                &log_user,
            );
            return Ok(());
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

    show_project(&project, &ctx.root);
    Ok(())
}

fn get_value(project: &Project, key: &str) -> Result<()> {
    match key {
        "name" => println!("{}", project.name),
        "acronym" => match &project.acronym {
            Some(a) => println!("{a}"),
            None => std::process::exit(1),
        },
        "description" => match &project.description {
            Some(d) => println!("{d}"),
            None => std::process::exit(1),
        },
        "language" => println!("{}", project.language),
        "created" => println!("{}", project.created.format("%Y-%m-%d %H:%M")),
        "docs.architecture" => println!("{}", project.docs.architecture_or_default()),
        "docs.vision" => println!("{}", project.docs.vision_or_default()),
        "docs.contributing" => println!("{}", project.docs.contributing_or_default()),
        _ => anyhow::bail!(
            "unknown key: {key}\nknown keys: {}",
            PROJECT_KEYS.join(", ")
        ),
    }
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

    let w = 12;
    if let Some(ref acronym) = project.acronym {
        println!("{}", color::key_value("Acronym:", acronym, w));
    }
    if let Some(ref description) = project.description {
        println!("{}", color::key_value("Description:", description, w));
    }
    println!("{}", color::key_value("Language:", &project.language, w));
    println!(
        "{}",
        color::key_value(
            "Created:",
            &project.created.format("%Y-%m-%d %H:%M").to_string(),
            w
        )
    );
    if !project.members.is_empty() {
        println!("\n{}:", color::label("Members"));
        print_members_table(&project.members, root);
    }

    // Workflow visualization with gates
    show_workflow(root);

    println!("{}", color::label(&"-".repeat(color::terminal_width())));

    // Hint about member modes if AI members exist
    if project.members.keys().any(|id| id.starts_with("ai:")) {
        println!(
            "{}",
            color::label("Use `joy project member show <ID>` to see interaction modes")
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
            // List members
            if project.members.is_empty() {
                println!("No members configured.");
            } else {
                print_members_table(&project.members, &ctx.root);
            }
        }
        Some(MemberCommand::Show(a)) => {
            let member = project
                .members
                .get(&a.id)
                .ok_or_else(|| anyhow::anyhow!("member not found: {}", a.id))?;

            let w = color::terminal_width();
            let wide = w >= 60;

            println!("{}", color::header(&a.id));

            // Load defaults for mode resolution
            let raw_defaults = joy_core::store::load_raw_mode_defaults(&ctx.root);
            let effective_defaults = joy_core::store::load_mode_defaults(&ctx.root);
            let config = joy_core::store::load_config();
            let personal_mode =
                if config.modes.default != joy_core::model::config::InteractionLevel::default() {
                    Some(config.modes.default)
                } else {
                    None
                };

            // Build capability list with has/denied and mode info
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
                    let (mode, source) = joy_core::model::project::resolve_mode(
                        cap,
                        &raw_defaults,
                        &effective_defaults,
                        personal_mode,
                        cap_config,
                    );
                    let mode_text = format!("{mode} [{source}]");
                    let mut line = if wide {
                        format!(
                            "  {:<12} {}   {}",
                            cap_label,
                            mark,
                            color::inactive(&mode_text)
                        )
                    } else {
                        format!(
                            "  {:<5} {}   {}",
                            cap_label,
                            mark,
                            color::inactive(&mode_text)
                        )
                    };
                    // Show max-mode hint if clamped
                    if source == joy_core::model::project::ModeSource::ProjectMax {
                        if let Some(personal) = personal_mode {
                            line.push_str(&color::inactive(&format!(
                                "  (your preference: {personal})"
                            )));
                        }
                    }
                    // Show max-mode from cap config
                    if let Some(cc) = cap_config {
                        if let Some(ref max) = cc.max_mode {
                            if source != joy_core::model::project::ModeSource::ProjectMax {
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
            if project.members.contains_key(&a.id) {
                bail!("member {} already exists", a.id);
            }
            let capabilities = match a.capabilities {
                None => MemberCapabilities::All,
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
            let attester_kp =
                derive_acting_keypair(project, &attester_email, a.passphrase.as_deref())?;

            // Generate a one-time password for the new member; the admin
            // shares it out-of-band, the new member redeems it via
            // `joy auth --otp` to set their passphrase (JOY-0072).
            let otp = joy_core::auth::otp::generate_otp();
            let otp_hash = joy_core::auth::otp::hash_otp(&otp)?;

            // Construct and sign the attestation over (email, capabilities,
            // otp_hash). public_key is intentionally not covered.
            let signed_fields = joy_core::auth::attestation::signed_fields_for(
                &a.id,
                &capabilities,
                Some(&otp_hash),
            );
            let attestation = joy_core::auth::attestation::sign_attestation(
                &attester_email,
                &attester_kp,
                signed_fields,
            );

            let mut new_member = Member::new(capabilities);
            new_member.otp_hash = Some(otp_hash);
            new_member.attestation = Some(attestation);
            project.members.insert(a.id.clone(), new_member);

            store::write_yaml_preserve(project_path, project)?;
            let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
            joy_core::git_ops::auto_git_add(&ctx.root, &[&rel]);

            println!("Added member {}", a.id);
            println!();
            println!("  One-time password: {otp}");
            println!();
            println!(
                "Share the OTP with {} via a trusted channel. They redeem it with:",
                a.id
            );
            println!("  joy auth --otp {otp}");

            let log_user = ctx.log_user();
            joy_core::git_ops::auto_git_post_command(
                &ctx.root,
                &format!("project member add {}", a.id),
                &log_user,
            );
        }
        Some(MemberCommand::Rm(a)) => {
            ctx.enforce(&Action::ManageProject, "project")?;
            // Prevent removing the last member with manage capability
            let guard = joy_core::guard::Guard::new(project);
            if guard.is_last_manager(&a.id) {
                bail!(
                    "cannot remove {}: last member with manage capability. \
                     Add another manage-capable member first.",
                    a.id
                );
            }
            if project.members.remove(&a.id).is_none() {
                bail!("member not found: {}", a.id);
            }
            store::write_yaml_preserve(project_path, project)?;
            let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
            joy_core::git_ops::auto_git_add(&ctx.root, &[&rel]);
            println!("Removed member {}", a.id);
            let log_user = ctx.log_user();
            joy_core::git_ops::auto_git_post_command(
                &ctx.root,
                &format!("project member rm {}", a.id),
                &log_user,
            );
        }
    }
    Ok(())
}

/// Derive and verify the acting human member's identity keypair from their
/// passphrase. Used to sign attestations on `joy project member add`.
fn derive_acting_keypair(
    project: &Project,
    email: &str,
    passphrase_flag: Option<&str>,
) -> Result<sign::IdentityKeypair> {
    let member = project
        .members
        .get(email)
        .ok_or_else(|| anyhow::anyhow!("{} is not a registered project member", email))?;
    let public_key_hex = member.public_key.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "{} has no registered public key. Run `joy auth init` first.",
            email
        )
    })?;
    let salt_hex = member.salt.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "{} has no registered salt. Run `joy auth init` first.",
            email
        )
    })?;
    let public_key = sign::PublicKey::from_hex(public_key_hex)?;
    let salt = derive::Salt::from_hex(salt_hex)?;
    let passphrase = match passphrase_flag {
        Some(p) => p.to_string(),
        None => rpassword::prompt_password("Passphrase: ")?,
    };
    let key = derive::derive_key(&passphrase, &salt)?;
    let keypair = sign::IdentityKeypair::from_derived_key(&key);
    if keypair.public_key() != public_key {
        anyhow::bail!("incorrect passphrase");
    }
    Ok(keypair)
}

fn print_members_table(
    members: &std::collections::BTreeMap<String, Member>,
    root: &std::path::Path,
) {
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
    let auth_statuses: Vec<(&str, String)> = members
        .iter()
        .map(|(id, member)| {
            let auth = member_auth_status(id, member, members, &project_id, use_emoji);
            (id.as_str(), auth)
        })
        .collect();

    let w_auth = auth_statuses
        .iter()
        .map(|(_, a)| display_width(a))
        .max()
        .unwrap_or(4)
        .max(4);

    let max_member = members.keys().map(|k| k.len()).max().unwrap_or(6).max(6);
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
    for ((id, member), (_, auth)) in members.iter().zip(auth_statuses.iter()) {
        let display_id = truncate(id, w_member);
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
    all_members: &std::collections::BTreeMap<String, Member>,
    project_id: &str,
    use_emoji: bool,
) -> String {
    use joy_core::model::project::is_ai_member;

    let is_ai = is_ai_member(id);

    // For humans: has passphrase key?
    // For AI: has any human registered an ai_delegations entry for this AI?
    let has_auth = if is_ai {
        all_members
            .values()
            .any(|m| m.ai_delegations.contains_key(id))
    } else {
        member.public_key.is_some()
    };

    // Session check: for humans, validate against their public_key.
    // For AI, just check if a session file exists and is not expired.
    let has_session = if !has_auth {
        false
    } else if is_ai {
        // AI sessions: check session exists, not expired, and its delegation
        // binding still matches a current ai_delegations entry. Rotating the
        // delegation invalidates any session bound to the previous key.
        let current_delegation_keys: Vec<&str> = all_members
            .values()
            .filter_map(|m| m.ai_delegations.get(id))
            .map(|entry| entry.delegation_key.as_str())
            .collect();
        joy_core::auth::session::load_session(project_id, id)
            .ok()
            .flatten()
            .and_then(|sess| {
                if sess.claims.expires <= chrono::Utc::now() || sess.claims.member != id {
                    let _ = joy_core::auth::session::remove_session(project_id, id);
                    return None;
                }
                match &sess.claims.token_key {
                    Some(tk) if current_delegation_keys.contains(&tk.as_str()) => Some(()),
                    Some(_) => {
                        // Delegation rotated — previous session is no longer trusted.
                        let _ = joy_core::auth::session::remove_session(project_id, id);
                        None
                    }
                    None => None,
                }
            })
            .is_some()
    } else if let Some(pk_hex) = member.public_key.as_ref() {
        if let Ok(pk) = joy_core::auth::sign::PublicKey::from_hex(pk_hex) {
            joy_core::auth::session::load_session(project_id, id)
                .ok()
                .flatten()
                .and_then(|token| {
                    joy_core::auth::session::validate_session(&token, &pk, project_id)
                        .ok()
                        .filter(|claims| claims.member == id)
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
