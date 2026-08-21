// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! joy-gitlab: the GitLab forge plugin (JOY-0255-B3, epic JOY-0251-AA).
//!
//! The second implementation of the forge query contract
//! (docs/plugins.md, "Forge plugins") — proof that the contract carries
//! without any joy-core change. All GitLab knowledge lives here: host
//! matching (gitlab.com; self-hosted instances are reached via the
//! project.yaml `forge:` override, a URL alone cannot identify them),
//! the noreply alias form `<id>-<username>@users.noreply.gitlab.com`,
//! glab's config, the API.

mod gitlab;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "joy-gitlab", about = "Joy forge plugin for GitLab")]
struct Cli {
    /// Run as if started in <PATH> (parity with joy's -w).
    #[arg(short = 'w', long, global = true)]
    working_dir: Option<std::path::PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Does this remote belong to GitLab?
    Claims {
        #[arg(long)]
        remote: String,
    },
    /// Who is ACTING on GitLab?
    Identity {
        #[arg(long)]
        login: Option<String>,
        #[arg(long)]
        user_id: Option<String>,
        /// Environment variable holding a GitLab token (never the token
        /// itself: it must not appear in a process list).
        #[arg(long)]
        token_env: Option<String>,
    },
    /// Whose address is this? Pure: answered from the address alone.
    Resolve {
        #[arg(long)]
        email: String,
    },
    /// Create the release for a tag (JOY-0256-64). GitLab has no release
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
            serde_json::json!({ "claims": gitlab::claims_remote(&remote) })
        }
        Command::Identity {
            login,
            user_id,
            token_env,
        } => gitlab::identity_answer(login, user_id, token_env.as_deref()),
        Command::Resolve { email } => gitlab::resolve_answer(&email),
        Command::Release { .. } => serde_json::json!({ "unsupported": true }),
    };
    println!("{}", serde_json::to_string(&answer)?);
    Ok(())
}
