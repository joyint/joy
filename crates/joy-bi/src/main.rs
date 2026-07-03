// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! joy-bi: the reference Joy plugin (JOY-01E8, app ADR JAPP-0088). A plugin
//! is a `joy-<name>` binary that computes on the project via joy-core and
//! prints ONE JoyNode tree as JSON on stdout; errors go to stderr with a
//! non-zero exit. Everything else (discovery, rendering, charts) is the
//! caller's job. docs/plugins.md is the author guide.

mod nodes;
mod report;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "joy-bi",
    about = "Joy BI reports as plugin nodes (JSON on stdout)"
)]
struct Cli {
    /// Run as if joy-bi was started in <PATH>
    #[arg(short = 'w', long, global = true)]
    working_dir: Option<std::path::PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Milestone report: progress, status and type breakdown
    Milestone {
        /// The milestone ID (e.g. JAPP-MS-01)
        id: String,
    },
    /// Closed items per bucket over a trailing window (h, d, w, m)
    Velocity {
        /// Window like 2w, 48h, 30d, 3m
        window: String,
    },
}

fn main() {
    let cli = Cli::parse();
    let start = cli
        .working_dir
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let Some(root) = joy_core::store::find_project_root(&start) else {
        eprintln!("joy-bi: no Joy project found (missing .joy/)");
        std::process::exit(2);
    };

    let result = match cli.command {
        Command::Milestone { id } => report::milestone(&root, &id),
        Command::Velocity { window } => report::velocity(&root, &window),
    };
    match result {
        Ok(node) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&node).expect("nodes serialize")
            );
        }
        Err(e) => {
            eprintln!("joy-bi: {e}");
            std::process::exit(1);
        }
    }
}
