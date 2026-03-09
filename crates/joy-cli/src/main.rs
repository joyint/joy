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
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Init(args)) => commands::init::run(args),
        None => {
            println!("joy v0.1.0 -- run `joy init` to get started");
            Ok(())
        }
    }
}
