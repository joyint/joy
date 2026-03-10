// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

mod color;
mod commands;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "joy", version, about = "Terminal-native product management")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new Joy project
    Init(commands::init::InitArgs),
    /// Create a new item
    Add(commands::add::AddArgs),
    /// List items
    Ls(commands::ls::LsArgs),
    /// Show item details
    Show(commands::show::ShowArgs),
    /// Modify an existing item
    Edit(commands::edit::EditArgs),
    /// Change item status
    Status(commands::status::StatusArgs),
    /// Delete an item
    Rm(commands::rm::RmArgs),
    /// Manage dependencies
    Deps(commands::deps::DepsArgs),
    /// Manage milestones
    Milestone(commands::milestone::MilestoneArgs),
    /// Assign or unassign items
    Assign(commands::assign::AssignArgs),
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Init(args)) => commands::init::run(args),
        Some(Commands::Add(args)) => commands::add::run(args),
        Some(Commands::Ls(args)) => commands::ls::run(args),
        Some(Commands::Show(args)) => commands::show::run(args),
        Some(Commands::Edit(args)) => commands::edit::run(args),
        Some(Commands::Status(args)) => commands::status::run(args),
        Some(Commands::Rm(args)) => commands::rm::run(args),
        Some(Commands::Deps(args)) => commands::deps::run(args),
        Some(Commands::Milestone(args)) => commands::milestone::run(args),
        Some(Commands::Assign(args)) => commands::assign::run(args),
        None => commands::board::run(),
    }
}
