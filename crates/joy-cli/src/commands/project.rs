// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use clap::Args;

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
            set_value(&mut project, &a.key, &a.value)?;
            store::write_yaml(&project_path, &project)?;
            println!("{} = {}", a.key, a.value);
            return Ok(());
        }
        None => {}
    }

    // Legacy flag-based editing
    let is_edit = args.name.is_some() || args.description.is_some() || args.language.is_some();

    if is_edit {
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
        println!("Project updated.");
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
    println!("{}", color::heading(&project.name));
    println!("{}", color::label(&"-".repeat(60)));

    if let Some(ref acronym) = project.acronym {
        println!("{} {}", color::label("Acronym:    "), acronym);
    }
    if let Some(ref description) = project.description {
        println!("{} {}", color::label("Description:"), description);
    }
    println!("{} {}", color::label("Language:   "), project.language);
    println!(
        "{} {}",
        color::label("Created:    "),
        color::label(&project.created.format("%Y-%m-%d %H:%M").to_string())
    );
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
