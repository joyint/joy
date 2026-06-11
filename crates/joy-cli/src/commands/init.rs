// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::io::IsTerminal;

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

    /// Start the project in anonymous privacy mode (ADR-042). The founder is
    /// recorded under an opaque id from the very first written file, so the
    /// git e-mail never lands in a committed project. Authentication is set up
    /// immediately, so a passphrase is required.
    #[arg(long)]
    pub anonymous: bool,

    /// Passphrase for the founder identity (only with --anonymous). Falls back
    /// to the JOY_PASSPHRASE env var, then an interactive prompt.
    #[arg(long)]
    pub passphrase: Option<String>,

    /// Read the founder passphrase from stdin (only with --anonymous).
    #[arg(long)]
    pub passphrase_stdin: bool,
}

pub fn run(args: InitArgs) -> Result<()> {
    let root = std::env::current_dir()?;
    let options = InitOptions {
        root: root.clone(),
        name: args.name,
        acronym: args.acronym,
        user: args.user.clone(),
        language: args.language,
    };

    // Anonymous mode is chosen by the --anonymous flag or, interactively, by a
    // Y/n prompt (the concept keeps both paths). The prompt is skipped for
    // --json and non-interactive runs (CI, tests), which must stay open unless
    // the flag is given.
    let want_anonymous = args.anonymous
        || (!crate::output::is_json()
            && std::io::stdin().is_terminal()
            && crate::prompt::ask_yn(
                "Start this project in anonymous privacy mode (ADR-042)?",
                false,
            )
            .unwrap_or(false));

    // For an anonymous start, acquire (and validate) the founder passphrase
    // BEFORE scaffolding. A failure here (no passphrase, no TTY, mismatch)
    // must leave nothing on disk -- otherwise init::init would have already
    // written an e-mail-keyed, open project.
    let anon_passphrase = if want_anonymous {
        Some(acquire_founder_passphrase(
            args.passphrase.as_deref(),
            args.passphrase_stdin,
        )?)
    } else {
        None
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
            // Render SECURITY.md so a fresh `joy update --check` is
            // clean immediately after init (otherwise the auth section
            // would report SECURITY.md as stale). project.yaml is just
            // created at the current schema, so no migration is needed.
            // Stage with the rest of joy's writes (JOY-0184-4A) so the
            // first commit picks it up alongside .joy/* and templates.
            if matches!(
                joy_core::security_md::render(&root.join("SECURITY.md")),
                Ok(true)
            ) {
                joy_core::git_ops::auto_git_add(&root, &["SECURITY.md"]);
            }

            // Anonymous onboarding (ADR-042). Done BEFORE the init auto-commit
            // below so the first committed project.yaml is already anonymous and
            // the git e-mail is never written to a committed file. run_init
            // establishes the founder identity, rekeys to the opaque id, writes
            // the encrypted members.yaml and stages the anonymized files.
            if let Some(ref pass) = anon_passphrase {
                println!();
                // Pass the pre-acquired passphrase as the flag so run_init runs
                // non-interactively and does not prompt a second time.
                crate::commands::auth::run_init(Some(pass), false, args.user.as_deref(), true)?;
            }

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

/// Acquire and validate the founder passphrase for `joy init --anonymous`,
/// before any project files are written. Honors `--passphrase`, then
/// `JOY_PASSPHRASE`, then an interactive prompt with confirmation.
fn acquire_founder_passphrase(flag: Option<&str>, from_stdin: bool) -> Result<String> {
    let env = std::env::var("JOY_PASSPHRASE")
        .ok()
        .filter(|s| !s.is_empty());
    let effective = flag.or(env.as_deref());
    let interactive = effective.is_none() && !from_stdin;
    if interactive {
        eprintln!("Starting an anonymous project (ADR-042).");
        eprintln!("Choose a founder passphrase (minimum 3 words, e.g. Diceware):");
    }
    let passphrase =
        crate::commands::auth::read_passphrase(effective, from_stdin, "  Passphrase: ")?;
    joy_core::auth::validate_passphrase(&passphrase)?;
    if interactive {
        let confirm = rpassword::prompt_password("  Confirm:    ")?;
        if passphrase != confirm {
            anyhow::bail!("passphrases do not match");
        }
    }
    Ok(passphrase)
}
