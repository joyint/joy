// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::{bail, Result};
use clap::Args;

use joy_core::model::item::Capability;
use joy_core::model::project::{CapabilityConfig, Member, MemberCapabilities};
use joy_core::model::Project;
use joy_core::store;

use crate::color;

const PROJECT_KEYS: &[&str] = &["name", "acronym", "description", "language", "created"];

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

    /// Override identity (email or ai:tool@joy).
    #[arg(long)]
    author: Option<String>,

    #[command(subcommand)]
    command: Option<ProjectCommand>,
}

#[derive(clap::Subcommand)]
enum ProjectCommand {
    /// Get a project value by key (name, acronym, description, language, created)
    Get(GetArgs),
    /// Set a project value by key (name, description, language)
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
}

#[derive(clap::Args)]
struct MemberRmArgs {
    /// Member ID (email or ai:tool@joy)
    id: String,
}

pub fn run(args: ProjectArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let project_path = store::joy_dir(&root).join(store::PROJECT_FILE);
    let mut project: Project = store::read_yaml(&project_path)?;

    match args.command {
        Some(ProjectCommand::Get(a)) => {
            return get_value(&project, &a.key);
        }
        Some(ProjectCommand::Set(a)) => {
            joy_core::guard::enforce(
                &root,
                &joy_core::guard::Action::ManageProject,
                "project",
                args.author.as_deref(),
            )?;
            set_value(&mut project, &a.key, &a.value)?;
            store::write_yaml_preserve(&project_path, &project)?;
            let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
            joy_core::git_ops::auto_git_add(&root, &[&rel]);
            println!("{} = {}", a.key, a.value);
            let log_user = joy_core::identity::resolve_identity(&root)
                .map(|id| id.log_user())
                .unwrap_or_default();
            joy_core::git_ops::auto_git_post_command(
                &root,
                &format!("project set {} {}", a.key, a.value),
                &log_user,
            );
            return Ok(());
        }
        Some(ProjectCommand::Member(a)) => {
            return run_member(
                a,
                &mut project,
                &project_path,
                &root,
                args.author.as_deref(),
            );
        }
        None => {}
    }

    // Legacy flag-based editing
    let is_edit = args.name.is_some() || args.description.is_some() || args.language.is_some();

    if is_edit {
        joy_core::guard::enforce(
            &root,
            &joy_core::guard::Action::ManageProject,
            "project",
            args.author.as_deref(),
        )?;
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
        joy_core::git_ops::auto_git_add(&root, &[&rel]);
        println!("Project updated.");
        let log_user = joy_core::identity::resolve_identity(&root)
            .map(|id| id.log_user())
            .unwrap_or_default();
        joy_core::git_ops::auto_git_post_command(&root, "project edit", &log_user);
    }

    show_project(&project, &root);
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
        "acronym" | "created" => {
            anyhow::bail!("'{key}' is read-only");
        }
        _ => anyhow::bail!(
            "unknown key: {key}\nknown keys: {}",
            PROJECT_KEYS.join(", ")
        ),
    }
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
}

fn run_member(
    args: MemberArgs,
    project: &mut Project,
    project_path: &std::path::Path,
    root: &std::path::Path,
    author: Option<&str>,
) -> Result<()> {
    match args.command {
        None => {
            // List members
            if project.members.is_empty() {
                println!("No members configured.");
            } else {
                print_members_table(&project.members, root);
            }
        }
        Some(MemberCommand::Show(a)) => {
            let member = project
                .members
                .get(&a.id)
                .ok_or_else(|| anyhow::anyhow!("member not found: {}", a.id))?;
            println!("{}", color::id(&a.id));
            match &member.capabilities {
                MemberCapabilities::All => {
                    println!("  Capabilities: all");
                }
                MemberCapabilities::Specific(map) => {
                    println!("  Capabilities:");
                    for (cap, config) in map {
                        let mut details = Vec::new();
                        if let Some(ref mode) = config.max_mode {
                            details.push(format!("max-mode: {mode}"));
                        }
                        if let Some(cost) = config.max_cost_per_job {
                            details.push(format!("max-cost-per-job: {cost:.2}"));
                        }
                        if details.is_empty() {
                            println!("    {cap}");
                        } else {
                            println!("    {} ({})", cap, details.join(", "));
                        }
                    }
                }
            }
        }
        Some(MemberCommand::Add(a)) => {
            joy_core::guard::enforce(
                root,
                &joy_core::guard::Action::ManageProject,
                "project",
                author,
            )?;
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
            project
                .members
                .insert(a.id.clone(), Member::new(capabilities));
            store::write_yaml_preserve(project_path, project)?;
            let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
            joy_core::git_ops::auto_git_add(root, &[&rel]);
            println!("Added member {}", a.id);
            let log_user = joy_core::identity::resolve_identity(root)
                .map(|id| id.log_user())
                .unwrap_or_default();
            joy_core::git_ops::auto_git_post_command(
                root,
                &format!("project member add {}", a.id),
                &log_user,
            );
        }
        Some(MemberCommand::Rm(a)) => {
            joy_core::guard::enforce(
                root,
                &joy_core::guard::Action::ManageProject,
                "project",
                author,
            )?;
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
            joy_core::git_ops::auto_git_add(root, &[&rel]);
            println!("Removed member {}", a.id);
            let log_user = joy_core::identity::resolve_identity(root)
                .map(|id| id.log_user())
                .unwrap_or_default();
            joy_core::git_ops::auto_git_post_command(
                root,
                &format!("project member rm {}", a.id),
                &log_user,
            );
        }
    }
    Ok(())
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
    // For AI: has any human registered an ai_tokens entry for this AI?
    let has_auth = if is_ai {
        all_members.values().any(|m| m.ai_tokens.contains_key(id))
    } else {
        member.public_key.is_some()
    };

    // Session check: for humans, validate against their public_key.
    // For AI, just check if a session file exists and is not expired.
    let has_session = if !has_auth {
        false
    } else if is_ai {
        // AI sessions: check session exists, not expired, and token_key matches
        // a current ai_tokens entry (invalidates sessions from revoked tokens)
        let current_token_keys: Vec<&str> = all_members
            .values()
            .filter_map(|m| m.ai_tokens.get(id))
            .map(|entry| entry.token_key.as_str())
            .collect();
        joy_core::auth::session::load_session(project_id, id)
            .ok()
            .flatten()
            .and_then(|sess| {
                if sess.claims.expires <= chrono::Utc::now() || sess.claims.member != id {
                    // Expired or wrong member — clean up
                    let _ = joy_core::auth::session::remove_session(project_id, id);
                    return None;
                }
                // Check token_key matches a current entry
                match &sess.claims.token_key {
                    Some(tk) if current_token_keys.contains(&tk.as_str()) => Some(()),
                    Some(_) => {
                        // Token was revoked/replaced — clean up stale session
                        let _ = joy_core::auth::session::remove_session(project_id, id);
                        None
                    }
                    None => None, // Old session format without token_key
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
