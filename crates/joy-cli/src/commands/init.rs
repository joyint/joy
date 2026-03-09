// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use clap::Args;

use joy_core::init::{self, InitOptions};

#[derive(Args)]
pub struct InitArgs {
    /// Project name
    #[arg(long)]
    name: Option<String>,

    /// Project acronym (2-4 uppercase letters)
    #[arg(long)]
    acronym: Option<String>,
}

pub fn run(args: InitArgs) -> Result<()> {
    let root = std::env::current_dir()?;
    let options = InitOptions {
        root,
        name: args.name,
        acronym: args.acronym,
    };
    let result = init::init(options)?;

    println!(
        "Initialized Joy project in {}",
        result.project_dir.display()
    );
    if result.git_initialized {
        println!("Initialized new Git repository.");
    }
    println!();
    println!("Get started:");
    println!("  joy add       Create an item");
    println!("  joy ls        List items");
    println!("  joy status    Change item status");
    println!("  joy           Board overview");

    Ok(())
}
