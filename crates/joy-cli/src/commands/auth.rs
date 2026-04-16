// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::Utc;
use clap::{Args, Subcommand};

use joy_core::auth::{consumed, delegation, derive, session, sign, token};
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
    /// Manage delegation tokens for AI members
    Token(TokenArgs),
}

#[derive(Args)]
struct ResetArgs {
    /// Member to reset (default: yourself). Requires manage capability.
    member: Option<String>,
}

#[derive(Args)]
struct TokenArgs {
    #[command(subcommand)]
    command: TokenCommand,
}

#[derive(Subcommand)]
enum TokenCommand {
    /// Create a delegation token for an AI member
    Add(TokenAddArgs),
    /// Revoke a delegation token for an AI member
    Rm(TokenRmArgs),
}

#[derive(Args)]
struct TokenAddArgs {
    /// AI member ID (e.g. ai:claude@joy)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_ai_member))]
    member: String,

    /// Token expiry in hours (default 2; ADR-033 issuance window)
    #[arg(long)]
    ttl: Option<i64>,
}

#[derive(Args)]
struct TokenRmArgs {
    /// AI member ID (e.g. ai:claude@joy)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_ai_member))]
    member: String,
}

pub fn run(args: AuthArgs) -> Result<()> {
    match args.command {
        Some(AuthCommand::Init) => run_init(args.passphrase.as_deref()),
        Some(AuthCommand::Status) => run_status(),
        Some(AuthCommand::Reset(a)) => run_reset(a, args.passphrase.as_deref()),
        Some(AuthCommand::Token(a)) => run_token(a, args.passphrase.as_deref()),
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

    // Look up the stable delegation entry for this AI member under the delegator (ADR-033).
    let ai_member_id = &delegation.claims.ai_member;
    let delegation_entry = human_member
        .ai_delegations
        .get(ai_member_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No delegation registered for {} by {}. Create one with `joy auth token add {}`.",
                ai_member_id,
                human,
                ai_member_id
            )
        })?;
    let delegation_pk = sign::PublicKey::from_hex(&delegation_entry.delegation_key)?;

    // ADR-033 replay protection: reject tokens we have already redeemed.
    // Must come BEFORE validate_token so a rewound clock cannot smuggle an
    // already-consumed token back in via expiry bypass.
    if let Ok(Some(redeemed_at)) = consumed::is_consumed(&delegation.claims.token_id) {
        anyhow::bail!(
            "Token already consumed (redeemed at {}). \
             Ask the human to issue a new one with: joy auth token add {}",
            redeemed_at.format("%Y-%m-%d %H:%M UTC"),
            delegation.claims.ai_member
        );
    }

    // Validate dual signatures + project + expiry
    let claims = token::validate_token(&delegation, &human_pk, &delegation_pk, project_id)?;

    // Mark consumed only after full validation so that invalid tokens do
    // not pollute the replay log.
    if let Err(e) = consumed::mark_consumed(&claims.token_id, claims.expires) {
        eprintln!("Warning: could not record consumed token: {e}");
    }

    // Verify the AI member is registered
    if !project.members.contains_key(&claims.ai_member) {
        anyhow::bail!(
            "AI member {} is not registered in this project.",
            claims.ai_member
        );
    }

    // ADR-033: ephemeral per-session keypair. The private key lives only in
    // the `JOY_SESSION` env var; the public key is recorded in the session
    // claims. Validation re-derives the public key from the env var and
    // requires a match, so sibling terminals without the env var cannot
    // reuse the session file.
    let ephemeral_keypair = sign::IdentityKeypair::from_random();
    let ephemeral_private = ephemeral_keypair.to_seed_bytes();
    let session_token = session::create_session_for_ai(
        &ephemeral_keypair,
        &claims.ai_member,
        project_id,
        None,
        &delegation_entry.delegation_key,
    );
    session::save_session(project_id, &session_token)?;

    // Output session handle for eval (stdout) -- SSH-agent pattern.
    // Status message goes to stderr so `eval $(joy auth --token ...)` works.
    let sid = session::session_id(project_id, &claims.ai_member);
    let env_value = session::encode_session_env(&sid, &ephemeral_private);
    println!("export JOY_SESSION={env_value}");

    // Persist the env value to any configured AI tool settings so fresh
    // subshells launched by the tool pick it up without needing to eval.
    if let Err(e) =
        crate::commands::ai::write_session_env_for_member(root, &claims.ai_member, &env_value)
    {
        eprintln!("Warning: could not update AI tool settings: {e}");
    }

    eprintln!(
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

    let identity =
        joy_core::identity::resolve_identity(&root).map_err(|e| anyhow::anyhow!("{e}"))?;

    if identity.authenticated {
        let project_id = session::project_id(&root)?;
        if let Ok(Some(sess)) = session::load_session(&project_id, &identity.member) {
            let remaining = sess.claims.expires - Utc::now();
            let hours = remaining.num_hours();
            let minutes = remaining.num_minutes() % 60;
            println!("Authenticated as {}.", identity.member);
            if let Some(ref delegated_by) = identity.delegated_by {
                println!("Delegated by {}.", delegated_by);
            }
            println!("Session expires in {}h {}m.", hours, minutes);
        } else {
            println!(
                "Authenticated as {} (session file missing).",
                identity.member
            );
        }
    } else {
        // Check if auth is initialized for this member
        let project = store::load_project(&root)?;
        let member = project.members.get(&identity.member);
        let has_auth = member.is_some_and(|m| m.public_key.is_some());
        if has_auth {
            println!(
                "No active session for {}. Run `joy auth` to authenticate.",
                identity.member
            );
        } else {
            println!("Authentication not initialized for {}.", identity.member);
            println!("Run `joy auth init` to set up.");
        }
        // Exit non-zero so shell scripts can gate on authentication state
        // (e.g. `if joy auth status; then ...`). The human-readable status
        // is still printed above before the failure.
        std::process::exit(1);
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
        joy_core::guard::enforce(&root, &joy_core::guard::Action::ManageProject, "project")?;
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

/// `joy auth token` — manage delegation tokens.
fn run_token(args: TokenArgs, passphrase_flag: Option<&str>) -> Result<()> {
    match args.command {
        TokenCommand::Add(a) => run_token_add(a, passphrase_flag),
        TokenCommand::Rm(a) => run_token_rm(a, passphrase_flag),
    }
}

/// `joy auth token add <ai-member>` — create a delegation token.
fn run_token_add(args: TokenAddArgs, passphrase_flag: Option<&str>) -> Result<()> {
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
    joy_core::guard::enforce(&root, &joy_core::guard::Action::ManageProject, "project")?;

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

    // ADR-033: stable per-(human, AI) delegation key.
    //   1. If project.yaml already has the public key AND the matching private
    //      key is present locally -> reuse both (no project.yaml write, no
    //      merge conflict class).
    //   2. If neither is present -> generate a fresh keypair, persist private
    //      to local state (0600), and insert the public key into project.yaml
    //      exactly once.
    //   3. If one side is present but not the other (state/yaml desync) ->
    //      bail with a clear instruction to rotate rather than silently
    //      papering over a half-broken state.
    let project_id = session::project_id(&root)?;
    let existing_public = member
        .ai_delegations
        .get(&args.member)
        .map(|e| e.delegation_key.clone());
    let existing_private = delegation::load_delegation_key(&project_id, &args.member)?;

    let (delegation_keypair, new_entry) = match (existing_public, existing_private) {
        (Some(pub_hex), Some(seed)) => {
            let kp = sign::IdentityKeypair::from_seed(&seed);
            if kp.public_key().to_hex() != pub_hex {
                anyhow::bail!(
                    "Local delegation private key for {} does not match the public key in project.yaml. \
                     Run `joy ai rotate {}` to generate a fresh pair.",
                    args.member,
                    args.member
                );
            }
            (kp, false)
        }
        (None, None) => {
            let kp = sign::IdentityKeypair::from_random();
            let seed = kp.to_seed_bytes();
            delegation::save_delegation_key(&project_id, &args.member, &seed)?;
            (kp, true)
        }
        (Some(_), None) => anyhow::bail!(
            "Delegation for {} is recorded in project.yaml but the local private key is missing. \
             Run `joy ai rotate {}` to generate a fresh pair on this machine.",
            args.member,
            args.member
        ),
        (None, Some(_)) => anyhow::bail!(
            "Local delegation key for {} exists but no entry is recorded in project.yaml. \
             Run `joy ai rotate {}` to resynchronise state.",
            args.member,
            args.member
        ),
    };

    // ADR-033: default issuance TTL is 2 hours. Tokens are single-use, so
    // the window only needs to cover realistic human-to-AI handover delay.
    const DEFAULT_TOKEN_TTL_HOURS: i64 = 2;
    let ttl = Some(
        args.ttl
            .map(chrono::Duration::hours)
            .unwrap_or_else(|| chrono::Duration::hours(DEFAULT_TOKEN_TTL_HOURS)),
    );
    let token_obj = token::create_token(
        &keypair,
        &delegation_keypair,
        &args.member,
        &email,
        &project_id,
        ttl,
    );

    // Persist the delegation public key on first issuance. Subsequent
    // issuances for the same (human, AI) pair produce no project.yaml
    // write since the key is stable (ADR-033).
    let project_path = store::joy_dir(&root).join(store::PROJECT_FILE);
    let mut project_mut: joy_core::model::project::Project = store::read_yaml(&project_path)?;
    if new_entry {
        if let Some(m) = project_mut.members.get_mut(&email) {
            m.ai_delegations.insert(
                args.member.clone(),
                joy_core::model::project::AiDelegationEntry {
                    delegation_key: delegation_keypair.public_key().to_hex(),
                    created: chrono::Utc::now(),
                    rotated: None,
                },
            );
        }
        store::write_yaml_preserve(&project_path, &project_mut)?;
        let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
        joy_core::git_ops::auto_git_add(&root, &[&rel]);
    }

    let encoded = token::encode_token(&token_obj);

    let hours = args.ttl.unwrap_or(DEFAULT_TOKEN_TTL_HOURS);
    println!("Delegation token for {}:", args.member);
    println!();
    println!("  {}", encoded);
    println!();
    println!("The AI redeems it with:");
    println!("  joy auth --token {}", encoded);
    println!();
    println!("Token is single-use and expires in {hours} hours.");

    Ok(())
}

/// `joy auth token rm <ai-member>` — revoke a delegation token.
fn run_token_rm(args: TokenRmArgs, passphrase_flag: Option<&str>) -> Result<()> {
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
        anyhow::bail!("{} is not a registered project member.", args.member);
    }

    // Guard: requires manage capability
    joy_core::guard::enforce(&root, &joy_core::guard::Action::ManageProject, "project")?;

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

    // Remove the delegation entry from project.yaml and the private key
    // file from local state.
    let project_path = store::joy_dir(&root).join(store::PROJECT_FILE);
    let mut project_mut: joy_core::model::project::Project = store::read_yaml(&project_path)?;
    let removed = project_mut
        .members
        .get_mut(&email)
        .map(|m| m.ai_delegations.remove(&args.member).is_some())
        .unwrap_or(false);

    if !removed {
        anyhow::bail!("No delegation registered for {} by {}.", args.member, email);
    }

    store::write_yaml_preserve(&project_path, &project_mut)?;
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    joy_core::git_ops::auto_git_add(&root, &[&rel]);

    let project_id = session::project_id(&root)?;
    delegation::remove_delegation_key(&project_id, &args.member)?;
    let _ = session::remove_session(&project_id, &args.member);

    println!("Delegation for {} revoked.", args.member);

    joy_core::git_ops::auto_git_post_command(
        &root,
        &format!("auth token rm {}", args.member),
        &email,
    );

    Ok(())
}
