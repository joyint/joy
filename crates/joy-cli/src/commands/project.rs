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
            joy_core::guard::enforce(&root, &joy_core::guard::Action::ManageProject, "project")?;
            set_value(&mut project, &a.key, &a.value)?;
            store::write_yaml(&project_path, &project)?;
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
            return run_member(a, &mut project, &project_path, &root);
        }
        None => {}
    }

    // Legacy flag-based editing
    let is_edit = args.name.is_some() || args.description.is_some() || args.language.is_some();

    if is_edit {
        joy_core::guard::enforce(&root, &joy_core::guard::Action::ManageProject, "project")?;
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
        store::write_yaml(&project_path, &project)?;
        let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
        joy_core::git_ops::auto_git_add(&root, &[&rel]);
        println!("Project updated.");
        let log_user = joy_core::identity::resolve_identity(&root)
            .map(|id| id.log_user())
            .unwrap_or_default();
        joy_core::git_ops::auto_git_post_command(&root, "project edit", &log_user);
    }

    show_project(&project);
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

fn show_project(project: &Project) {
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
        print_members_table(&project.members);
    }
    println!("{}", color::label(&"-".repeat(color::terminal_width())));
}

fn run_member(
    args: MemberArgs,
    project: &mut Project,
    project_path: &std::path::Path,
    root: &std::path::Path,
) -> Result<()> {
    match args.command {
        None => {
            // List members
            if project.members.is_empty() {
                println!("No members configured.");
            } else {
                print_members_table(&project.members);
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
            joy_core::guard::enforce(root, &joy_core::guard::Action::ManageProject, "project")?;
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
                .insert(a.id.clone(), Member { capabilities });
            store::write_yaml(project_path, project)?;
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
            joy_core::guard::enforce(root, &joy_core::guard::Action::ManageProject, "project")?;
            if project.members.remove(&a.id).is_none() {
                bail!("member not found: {}", a.id);
            }
            store::write_yaml(project_path, project)?;
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

fn print_members_table(members: &std::collections::BTreeMap<String, Member>) {
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

    // Table needs: member column + 4 chars per capability column (3 + space) + padding
    let w_member = members.keys().map(|k| k.len()).max().unwrap_or(6).max(6);
    let table_width = 2 + w_member + 1 + cap_headers.len() * 4;
    let term_width = color::terminal_width();

    if table_width <= term_width {
        // Wide: capability matrix
        print!(
            "  {}",
            color::inactive(&format!("{:<w$}", "Member", w = w_member))
        );
        for (hdr, _) in cap_headers {
            print!(" {}", color::inactive(&format!("{:<3}", hdr)));
        }
        println!();

        for (id, member) in members {
            print!("  {:<w$}", id, w = w_member);
            for (_, cap) in cap_headers {
                let has = match &member.capabilities {
                    MemberCapabilities::All => true,
                    MemberCapabilities::Specific(map) => map.contains_key(cap),
                };
                if has {
                    print!("  x ");
                } else {
                    print!("    ");
                }
            }
            println!();
        }
    } else {
        // Narrow: comma-separated
        for (id, member) in members {
            let caps = match &member.capabilities {
                MemberCapabilities::All => "all".to_string(),
                MemberCapabilities::Specific(map) => {
                    let names: Vec<String> = map.keys().map(|c| c.to_string()).collect();
                    names.join(", ")
                }
            };
            println!("  {}  {}", id, caps);
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
