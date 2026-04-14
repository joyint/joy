// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use clap::Args;

use joy_core::error::JoyError;
use joy_core::init::{self, InitOptions};

#[derive(Args)]
#[command(after_help = "\
Creates a .joy/ directory in the current folder with:
  items/         Item storage (YAML files)
  project.yaml   Project metadata (name, acronym)

The acronym is used as prefix for all item and milestone IDs
(e.g. JOY-0001, JOY-MS-01). It defaults to the project name if omitted.
If no git repository exists, one is initialized.

If the project is already initialized, sets up your local environment
(git hooks, etc.) without modifying project data.")]
pub struct InitArgs {
    /// Project name (defaults to directory name)
    #[arg(long)]
    pub name: Option<String>,

    /// Project acronym (2-4 uppercase letters, derived from name if omitted)
    #[arg(long)]
    pub acronym: Option<String>,

    /// Creator member email (defaults to git config user.email)
    #[arg(long)]
    pub user: Option<String>,

    /// Project language (ISO 639-1 code, e.g. en, de). Defaults to en.
    #[arg(long)]
    pub language: Option<String>,
}

pub fn run(args: InitArgs) -> Result<()> {
    let root = std::env::current_dir()?;
    let options = InitOptions {
        root: root.clone(),
        name: args.name,
        acronym: args.acronym,
        user: args.user,
        language: args.language,
    };

    match init::init(options) {
        Ok(result) => {
            println!(
                "Initialized Joy project in {}",
                result.project_dir.display()
            );
            if result.git_initialized {
                println!("Initialized new Git repository.");
            }
            println!("Commit-msg hook installed.");
            println!();
            println!("Get started:");
            println!("  joy add <TYPE> <TITLE>   Create an item");
            println!("  joy ls                   List items");
            println!("  joy status <ID> <STATUS> Change item status");
            println!("  joy                      Board overview");
            println!();
            println!("Using AI tools? Run 'joy ai init' to configure integration.");
            let log_user = joy_core::identity::resolve_identity(&root)
                .map(|id| id.log_user())
                .unwrap_or_default();
            joy_core::git_ops::auto_git_post_command(&root, "init", &log_user);
        }
        Err(JoyError::AlreadyInitialized(_)) => {
            println!("Project already initialized. Setting up local environment...");
            let result = init::onboard(&root)?;
            if result.hooks_already_set {
                println!("  Commit-msg hook ... up to date");
            } else {
                println!("  Commit-msg hook ... installed");
            }
            println!();
            println!("Local environment ready.");
        }
        Err(e) => return Err(e.into()),
    }

    Ok(())
}
