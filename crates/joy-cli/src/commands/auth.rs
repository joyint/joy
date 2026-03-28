// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::Utc;
use clap::{Args, Subcommand};

use joy_core::auth::{derive, session, sign, token};
use joy_core::store;
use joy_core::vcs::Vcs;

use crate::color;

#[derive(Args)]
pub struct AuthArgs {
    #[command(subcommand)]
    command: Option<AuthCommand>,

    /// Passphrase (non-interactive, for scripts and tests).
    #[arg(long, global = true)]
    passphrase: Option<String>,

    /// Delegation token for AI authentication (alternative to JOY_TOKEN env var).
    #[arg(long, global = true)]
    token: Option<String>,
}

#[derive(Subcommand)]
enum AuthCommand {
    /// Initialize authentication: generate salt, derive keypair, register public key
    Init,
    /// Show current session status
    Status,
    /// Reset authentication (remove public key, salt, and session)
    Reset(ResetArgs),
    /// Create a delegation token for an AI member
    CreateToken(CreateTokenArgs),
}

#[derive(Args)]
struct ResetArgs {
    /// Member to reset (default: yourself). Requires manage capability.
    member: Option<String>,
}

#[derive(Args)]
struct CreateTokenArgs {
    /// AI member ID (e.g. ai:claude@joy)
    member: String,

    /// Token expiry in hours (default: no expiry)
    #[arg(long)]
    ttl: Option<i64>,
}

pub fn run(args: AuthArgs) -> Result<()> {
    match args.command {
        Some(AuthCommand::Init) => run_init(args.passphrase.as_deref()),
        Some(AuthCommand::Status) => run_status(),
        Some(AuthCommand::Reset(a)) => run_reset(a, args.passphrase.as_deref()),
        Some(AuthCommand::CreateToken(a)) => run_create_token(a, args.passphrase.as_deref()),
        None => run_auth(args.passphrase.as_deref(), args.token.as_deref()),
    }
}

/// Resolve token from --token flag or JOY_TOKEN env var.
fn resolve_token(flag: Option<&str>) -> Option<String> {
    flag.map(|s| s.to_string())
        .or_else(|| std::env::var("JOY_TOKEN").ok().filter(|s| !s.is_empty()))
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

    store::write_yaml_preserve(&project_path, &project)?;
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    joy_core::git_ops::auto_git_add(&root, &[&rel]);

    // Create initial session
    let project_id = session::project_id(&root)?;
    let session_token = session::create_session(&keypair, &email, &project_id, None);
    session::save_session(&project_id, &session_token)?;

    println!("Authentication initialized for {}.", email);
    println!("Public key registered. Session active (24h).");

    joy_core::git_ops::auto_git_post_command(&root, "auth init", &email);

    Ok(())
}

/// `joy auth` — authenticate by passphrase (human) or delegation token (AI).
fn run_auth(passphrase_flag: Option<&str>, token_flag: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let project = store::load_project(&root)?;
    let project_id = session::project_id(&root)?;

    // Check for delegation token (--token flag or JOY_TOKEN env var)
    if let Some(token_str) = resolve_token(token_flag) {
        return auth_with_token(&root, &project, &project_id, &token_str);
    }

    // Human authentication via passphrase
    let email = joy_core::vcs::default_vcs().user_email()?;
    auth_with_passphrase(&root, &project, &project_id, &email, passphrase_flag)
}

/// Authenticate a human member via passphrase.
fn auth_with_passphrase(
    _root: &std::path::Path,
    project: &joy_core::model::project::Project,
    project_id: &str,
    email: &str,
    passphrase_flag: Option<&str>,
) -> Result<()> {
    let member = project.members.get(email).ok_or_else(|| {
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

    let passphrase = read_passphrase(passphrase_flag, "Passphrase: ")?;
    let key = derive::derive_key(&passphrase, &salt)?;
    let keypair = sign::IdentityKeypair::from_derived_key(&key);

    if keypair.public_key() != public_key {
        anyhow::bail!("incorrect passphrase");
    }

    let session_token = session::create_session(&keypair, email, project_id, None);
    session::save_session(project_id, &session_token)?;

    println!("Authenticated as {}. Session active (24h).", email);

    Ok(())
}

/// Authenticate an AI member via delegation token.
fn auth_with_token(
    root: &std::path::Path,
    project: &joy_core::model::project::Project,
    project_id: &str,
    token_str: &str,
) -> Result<()> {
    // Decode the delegation token
    let delegation = token::decode_token(token_str)?;

    // Look up the delegating human
    let human = &delegation.claims.delegated_by;
    let human_member = project
        .members
        .get(human)
        .ok_or_else(|| anyhow::anyhow!("Delegating member {} is not registered.", human))?;
    let human_pk_hex = human_member.public_key.as_ref().ok_or_else(|| {
        anyhow::anyhow!("Delegating member {} has no public key registered.", human)
    })?;
    let human_pk = sign::PublicKey::from_hex(human_pk_hex)?;

    // Look up the ai_tokens entry for this AI member under the delegator
    let ai_member_id = &delegation.claims.ai_member;
    let token_entry = human_member.ai_tokens.get(ai_member_id).ok_or_else(|| {
        anyhow::anyhow!(
            "No token registered for {} by {}. Create one with `joy auth create-token {}`.",
            ai_member_id,
            human,
            ai_member_id
        )
    })?;
    let token_pk = sign::PublicKey::from_hex(&token_entry.token_key)?;

    // Validate dual signatures + project + expiry
    let claims = token::validate_token(&delegation, &human_pk, &token_pk, project_id)?;

    // Verify the AI member is registered
    if !project.members.contains_key(&claims.ai_member) {
        anyhow::bail!(
            "AI member {} is not registered in this project.",
            claims.ai_member
        );
    }

    // Create a local session — no project.yaml changes needed
    let session_keypair = sign::IdentityKeypair::from_token_seed(token_str, project_id);
    let session_token = session::create_session_for_ai(
        &session_keypair,
        &claims.ai_member,
        project_id,
        None,
        &token_entry.token_key,
    );
    session::save_session(project_id, &session_token)?;

    println!(
        "Authenticated as {} (delegated by {}). Session active (24h).",
        claims.ai_member, claims.delegated_by
    );

    joy_core::event_log::log_event_as(
        root,
        joy_core::event_log::EventType::AuthSessionCreated,
        "auth",
        Some(&format!(
            "session created for {} via delegation token",
            claims.ai_member
        )),
        &format!("{} delegated-by:{}", claims.ai_member, claims.delegated_by),
    );

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
    match session::load_session(&project_id, &email)? {
        Some(sess) => {
            let public_key_hex = member.unwrap().public_key.as_ref().unwrap();
            let public_key = sign::PublicKey::from_hex(public_key_hex)?;
            match session::validate_session(&sess, &public_key, &project_id) {
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

/// `joy auth reset [member]` — reset authentication for yourself or another member.
fn run_reset(args: ResetArgs, passphrase_flag: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let project_path = store::joy_dir(&root).join(store::PROJECT_FILE);
    let mut project: joy_core::model::project::Project = store::read_yaml(&project_path)?;
    let email = joy_core::vcs::default_vcs().user_email()?;

    let target = args.member.as_deref().unwrap_or(&email);
    let resetting_other = target != email;

    // Verify the acting user's identity via passphrase
    let acting_member = project
        .members
        .get(&email)
        .ok_or_else(|| anyhow::anyhow!("{} is not a registered project member.", email))?;

    if acting_member.public_key.is_none() {
        anyhow::bail!(
            "Authentication not initialized for {}. Run `joy auth init`.",
            email
        );
    }

    // Authenticate the acting user
    let salt_hex = acting_member.salt.as_ref().unwrap();
    let public_key_hex = acting_member.public_key.as_ref().unwrap();
    let salt = derive::Salt::from_hex(salt_hex)?;
    let public_key = sign::PublicKey::from_hex(public_key_hex)?;

    let passphrase = read_passphrase(passphrase_flag, "Passphrase: ")?;
    let key = derive::derive_key(&passphrase, &salt)?;
    let keypair = sign::IdentityKeypair::from_derived_key(&key);
    if keypair.public_key() != public_key {
        anyhow::bail!("incorrect passphrase");
    }

    // If resetting another member, check manage capability
    if resetting_other {
        joy_core::guard::enforce(
            &root,
            &joy_core::guard::Action::ManageProject,
            "project",
            None,
        )?;
    }

    // Verify target member exists
    if !project.members.contains_key(target) {
        anyhow::bail!("member not found: {}", target);
    }

    // Reset target member's auth fields
    let m = project.members.get_mut(target).unwrap();
    m.public_key = None;
    m.salt = None;
    m.otp_hash = None;

    store::write_yaml_preserve(&project_path, &project)?;
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    joy_core::git_ops::auto_git_add(&root, &[&rel]);

    // Remove own session if resetting self
    let project_id = session::project_id(&root)?;
    if !resetting_other {
        session::remove_session(&project_id, target)?;
    }

    println!("Authentication reset for {}.", target);
    if resetting_other {
        println!("They can re-initialize with `joy auth init`.");
    } else {
        println!("Run `joy auth init` to set up again.");
    }

    joy_core::git_ops::auto_git_post_command(&root, &format!("auth reset {}", target), &email);

    Ok(())
}

/// `joy auth create-token <ai-member>` — create a delegation token.
fn run_create_token(args: CreateTokenArgs, passphrase_flag: Option<&str>) -> Result<()> {
    use joy_core::model::project::is_ai_member;

    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let project = store::load_project(&root)?;
    let email = joy_core::vcs::default_vcs().user_email()?;

    // Validate AI member
    if !is_ai_member(&args.member) {
        anyhow::bail!("{} is not an AI member (must start with ai:)", args.member);
    }
    if !project.members.contains_key(&args.member) {
        anyhow::bail!(
            "{} is not a registered project member. Run `joy project member add {}`.",
            args.member,
            args.member
        );
    }

    // Guard: requires manage capability
    joy_core::guard::enforce(
        &root,
        &joy_core::guard::Action::ManageProject,
        "project",
        None,
    )?;

    // Authenticate the acting human
    let member = project
        .members
        .get(&email)
        .ok_or_else(|| anyhow::anyhow!("{} is not a registered project member.", email))?;
    if member.public_key.is_none() {
        anyhow::bail!(
            "Authentication not initialized for {}. Run `joy auth init`.",
            email
        );
    }

    let salt_hex = member.salt.as_ref().unwrap();
    let public_key_hex = member.public_key.as_ref().unwrap();
    let salt = derive::Salt::from_hex(salt_hex)?;
    let public_key = sign::PublicKey::from_hex(public_key_hex)?;

    let passphrase = read_passphrase(passphrase_flag, "Passphrase: ")?;
    let key = derive::derive_key(&passphrase, &salt)?;
    let keypair = sign::IdentityKeypair::from_derived_key(&key);
    if keypair.public_key() != public_key {
        anyhow::bail!("incorrect passphrase");
    }

    // Create token
    let project_id = session::project_id(&root)?;
    let ttl = args.ttl.map(chrono::Duration::hours);
    let result = token::create_token(&keypair, &args.member, &email, &project_id, ttl);

    // Store token_key in delegator's ai_tokens
    let project_path = store::joy_dir(&root).join(store::PROJECT_FILE);
    let mut project_mut: joy_core::model::project::Project = store::read_yaml(&project_path)?;
    if let Some(m) = project_mut.members.get_mut(&email) {
        m.ai_tokens.insert(
            args.member.clone(),
            joy_core::model::project::AiTokenEntry {
                token_key: result.token_public_key.clone(),
                created: chrono::Utc::now(),
            },
        );
    }
    store::write_yaml_preserve(&project_path, &project_mut)?;
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    joy_core::git_ops::auto_git_add(&root, &[&rel]);

    let encoded = token::encode_token(&result.token);

    println!("Delegation token for {}:", args.member);
    println!();
    println!("  {}", encoded);
    println!();
    println!("The AI agent authenticates with:");
    println!("  export JOY_TOKEN={}", encoded);
    println!("  joy auth");
    println!();
    println!("Or in a single command:");
    println!("  joy auth --token {}", encoded);
    if let Some(hours) = args.ttl {
        println!("Token expires in {hours} hours.");
    } else {
        println!("Token does not expire. Revoke by resetting the AI member's auth.");
    }

    Ok(())
}
