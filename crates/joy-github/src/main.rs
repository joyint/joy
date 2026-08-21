// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! joy-github: the GitHub forge plugin (JOY-0254-3C, epic JOY-0251-AA).
//!
//! Speaks the forge query contract (docs/plugins.md, "Forge plugins"):
//! `claims --remote <url>` and `identity [...]`, one JSON object on
//! stdout. ALL GitHub knowledge lives here — host matching, the noreply
//! alias address forms, gh's config, the API — and nowhere else.
//!
//! Facts, in order of authority:
//! - handed-in caller facts (`--login/--user-id`, a multi-account host's
//!   session) win over anything discovered locally;
//! - the gh CLI's config (`~/.config/gh/hosts.yml`) names the signed-in
//!   login, offline;
//! - a `--token-env`/gh-authenticated API call lists the account's
//!   verified addresses (best effort: without the user:email scope the
//!   list stays empty and the answer still names the login).

mod github;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "joy-github", about = "Joy forge plugin for GitHub")]
struct Cli {
    /// Run as if started in <PATH> (parity with joy's -w).
    #[arg(short = 'w', long, global = true)]
    working_dir: Option<std::path::PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Does this remote belong to GitHub?
    Claims {
        #[arg(long)]
        remote: String,
    },
    /// Who is ACTING on GitHub?
    Identity {
        /// Caller facts from a multi-account host (win over discovery).
        #[arg(long)]
        login: Option<String>,
        #[arg(long)]
        user_id: Option<String>,
        /// Environment variable holding a GitHub token (never the token
        /// itself: it must not appear in a process list).
        #[arg(long)]
        token_env: Option<String>,
    },
    /// Whose address is this? Pure: answered from the address alone.
    Resolve {
        #[arg(long)]
        email: String,
    },
    /// Create (or complete) the release for a tag on GitHub
    /// (JOY-0256-64). Unlike the read queries this reports failures:
    /// stderr carries the reason, the exit code is non-zero.
    Release {
        #[arg(long)]
        tag: String,
        #[arg(long)]
        title: String,
        /// The release notes, passed as a file (they are multi-line and
        /// may be long).
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
            serde_json::json!({ "claims": github::claims_remote(&remote) })
        }
        Command::Identity {
            login,
            user_id,
            token_env,
        } => github::identity_answer(login, user_id, token_env.as_deref()),
        Command::Resolve { email } => github::resolve_answer(&email),
        Command::Release {
            tag,
            title,
            notes_file,
        } => {
            let notes = std::fs::read_to_string(&notes_file)?;
            github::release_answer(&tag, &title, &notes)?
        }
    };
    println!("{}", serde_json::to_string(&answer)?);
    Ok(())
}
