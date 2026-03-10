// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

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
    /// Change item status
    Status(commands::status::StatusArgs),
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Init(args)) => commands::init::run(args),
        Some(Commands::Add(args)) => commands::add::run(args),
        Some(Commands::Ls(args)) => commands::ls::run(args),
        Some(Commands::Status(args)) => commands::status::run(args),
        None => commands::board::run(),
    }
}
