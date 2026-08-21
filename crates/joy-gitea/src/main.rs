// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! joy-gitea: the Gitea forge plugin (JOY-025B-F6, epic JOY-0251-AA).
//!
//! The third implementation of the forge query contract
//! (docs/plugins.md, "Forge plugins"). All Gitea knowledge lives here:
//! host matching (Gitea and Forgejo are self-hosted with no canonical
//! domain, so a remote is claimed when tea is signed in to that host;
//! everything else takes the project.yaml `forge:` override), the
//! noreply alias form `<username>@noreply.<instance>`, tea's config,
//! the API.

mod gitea;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "joy-gitea", about = "Joy forge plugin for Gitea")]
struct Cli {
    /// Run as if started in <PATH> (parity with joy's -w).
    #[arg(short = 'w', long, global = true)]
    working_dir: Option<std::path::PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Does this remote belong to Gitea?
    Claims {
        #[arg(long)]
        remote: String,
    },
    /// Who is ACTING on Gitea?
    Identity {
        #[arg(long)]
        login: Option<String>,
        #[arg(long)]
        user_id: Option<String>,
        /// Environment variable holding a Gitea token (never the token
        /// itself: it must not appear in a process list).
        #[arg(long)]
        token_env: Option<String>,
    },
    /// Whose address is this? Pure: answered from the address alone.
    Resolve {
        #[arg(long)]
        email: String,
    },
    /// Create the release for a tag (JOY-0256-64). Gitea has no release
    /// backend yet: the honest answer is `unsupported`, and joy keeps
    /// the tag-only publish instead of failing the release.
    Release {
        #[arg(long)]
        tag: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        notes_file: std::path::PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if let Some(dir) = &cli.working_dir {
        std::env::set_current_dir(dir)?;
    }
    let answer = match cli.command {
        Command::Claims { remote } => {
            serde_json::json!({ "claims": gitea::claims_remote(&remote) })
        }
        Command::Identity {
            login,
            user_id,
            token_env,
        } => gitea::identity_answer(login, user_id, token_env.as_deref()),
        Command::Resolve { email } => gitea::resolve_answer(&email),
        Command::Release { .. } => serde_json::json!({ "unsupported": true }),
    };
    println!("{}", serde_json::to_string(&answer)?);
    Ok(())
}
