// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use clap::Args;

use joy_core::model::Project;
use joy_core::store;

use crate::color;

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
}

pub fn run(args: ProjectArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let project_path = store::joy_dir(&root).join(store::PROJECT_FILE);
    let mut project: Project = store::read_yaml(&project_path)?;

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

    Ok(())
}
