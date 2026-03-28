// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::Utc;
use clap::{Args, Subcommand};

use joy_core::auth::{derive, session, sign};
use joy_core::store;
use joy_core::vcs::Vcs;

use crate::color;

#[derive(Args)]
pub struct AuthArgs {
    #[command(subcommand)]
    command: Option<AuthCommand>,

    /// Passphrase (non-interactive, for scripts and tests).
    /// If not set, prompts interactively.
    #[arg(long, global = true)]
    passphrase: Option<String>,
}

#[derive(Subcommand)]
enum AuthCommand {
    /// Initialize authentication: generate salt, derive keypair, register public key
    Init,
    /// Show current session status
    Status,
}

pub fn run(args: AuthArgs) -> Result<()> {
    match args.command {
        Some(AuthCommand::Init) => run_init(args.passphrase.as_deref()),
        Some(AuthCommand::Status) => run_status(),
        None => run_auth(args.passphrase.as_deref()),
    }
}

/// Read passphrase from flag or prompt interactively.
fn read_passphrase(flag: Option<&str>, prompt: &str) -> Result<String> {
    match flag {
        Some(p) => Ok(p.to_string()),
        None => Ok(rpassword::prompt_password(prompt)?),
    }
}

/// `joy auth init` — first-time setup for the current member.
fn run_init(passphrase_flag: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let project_path = store::joy_dir(&root).join(store::PROJECT_FILE);
    let mut project: joy_core::model::project::Project = store::read_yaml(&project_path)?;

    // Determine who we are
    let email = joy_core::vcs::default_vcs().user_email()?;
    let member = project.members.get(&email);
    if member.is_none() {
        anyhow::bail!(
            "{} is not a registered project member. Run `joy project member add {}`.",
            email,
            email
        );
    }
    let member = member.unwrap();
    if member.public_key.is_some() {
        anyhow::bail!(
            "{} already has authentication initialized. Use `joy auth` to authenticate.",
            email
        );
    }

    // Get passphrase
    if passphrase_flag.is_none() {
        eprintln!("Setting up authentication for {}.", color::id(&email));
        eprintln!("Choose a passphrase (minimum 6 words, e.g. Diceware):");
    }
    let passphrase = read_passphrase(passphrase_flag, "  Passphrase: ")?;
    derive::validate_passphrase(&passphrase)?;

    // Confirm (only in interactive mode)
    if passphrase_flag.is_none() {
        let confirm = rpassword::prompt_password("  Confirm:    ")?;
        if passphrase != confirm {
            anyhow::bail!("passphrases do not match");
        }
    }

    // Derive keypair
    let salt = derive::generate_salt();
    let key = derive::derive_key(&passphrase, &salt)?;
    let keypair = sign::IdentityKeypair::from_derived_key(&key);
    let public_key = keypair.public_key();

    // Store salt and public key in project.yaml
    let m = project.members.get_mut(&email).unwrap();
    m.salt = Some(salt.to_hex());
    m.public_key = Some(public_key.to_hex());

    store::write_yaml(&project_path, &project)?;
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    joy_core::git_ops::auto_git_add(&root, &[&rel]);

    // Create initial session
    let project_id = session::project_id(&root)?;
    let token = session::create_session(&keypair, &email, &project_id, None);
    session::save_session(&project_id, &token)?;

    println!("Authentication initialized for {}.", email);
    println!("Public key registered. Session active (24h).");

    joy_core::git_ops::auto_git_post_command(&root, "auth init", &email);

    Ok(())
}

/// `joy auth` — authenticate by entering passphrase.
fn run_auth(passphrase_flag: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let project = store::load_project(&root)?;
    let email = joy_core::vcs::default_vcs().user_email()?;

    let member = project.members.get(&email).ok_or_else(|| {
        anyhow::anyhow!(
            "{} is not a registered project member. Run `joy project member add {}`.",
            email,
            email
        )
    })?;

    let public_key_hex = member.public_key.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "Authentication not initialized for {}. Run `joy auth init`.",
            email
        )
    })?;
    let salt_hex = member
        .salt
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No salt found for {}. Run `joy auth init`.", email))?;

    let public_key = sign::PublicKey::from_hex(public_key_hex)?;
    let salt = derive::Salt::from_hex(salt_hex)?;

    // Get passphrase
    let passphrase = read_passphrase(passphrase_flag, "Passphrase: ")?;

    // Derive key and verify
    let key = derive::derive_key(&passphrase, &salt)?;
    let keypair = sign::IdentityKeypair::from_derived_key(&key);

    if keypair.public_key() != public_key {
        anyhow::bail!("incorrect passphrase");
    }

    // Create session
    let project_id = session::project_id(&root)?;
    let token = session::create_session(&keypair, &email, &project_id, None);
    session::save_session(&project_id, &token)?;

    println!("Authenticated as {}. Session active (24h).", email);

    Ok(())
}

/// `joy auth status` — show current session state.
fn run_status() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let project = store::load_project(&root)?;
    let email = joy_core::vcs::default_vcs().user_email()?;
    let project_id = session::project_id(&root)?;

    // Check if auth is initialized for this member
    let member = project.members.get(&email);
    let has_auth = member.is_some_and(|m| m.public_key.is_some());

    if !has_auth {
        println!("Authentication not initialized for {}.", email);
        println!("Run `joy auth init` to set up.");
        return Ok(());
    }

    // Check session
    match session::load_session(&project_id)? {
        Some(token) => {
            let public_key_hex = member.unwrap().public_key.as_ref().unwrap();
            let public_key = sign::PublicKey::from_hex(public_key_hex)?;
            match session::validate_session(&token, &public_key, &project_id) {
                Ok(claims) => {
                    let remaining = claims.expires - Utc::now();
                    let hours = remaining.num_hours();
                    let minutes = remaining.num_minutes() % 60;
                    println!("Authenticated as {}.", claims.member);
                    println!("Session expires in {}h {}m.", hours, minutes);
                }
                Err(_) => {
                    println!("Session expired. Run `joy auth` to re-authenticate.");
                }
            }
        }
        None => {
            println!("No active session. Run `joy auth` to authenticate.");
        }
    }

    Ok(())
}
