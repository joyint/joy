// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::Utc;
use clap::{Args, Subcommand};

use joy_core::auth::{
    delegation, derive_key, generate_salt, seed as seed_mod, session, token, validate_passphrase,
    IdentityKeypair, PublicKey, Salt,
};
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

    /// One-time password for first-time member setup (JOY-0072). Redeems
    /// the OTP, derives the caller's keypair from --passphrase, clears
    /// the stored otp_hash, and establishes an initial session.
    #[arg(long, global = true)]
    otp: Option<String>,

    /// Member ID to authenticate as. Overrides the git-email lookup so
    /// projects whose member entry differs from `git config user.email`
    /// (e.g. registered via `joy init --user`) can authenticate.
    #[arg(long, global = true)]
    user: Option<String>,
}

#[derive(Subcommand)]
enum AuthCommand {
    /// Initialize authentication: generate kdf_nonce, derive keypair, register verify_key
    Init,
    /// Show current session status
    Status,
    /// Reset authentication (remove verify_key, kdf_nonce, and session)
    Reset(ResetArgs),
    /// Manage delegation tokens for AI members
    Token(TokenArgs),
    /// Manage your per-(operator, AI) delegations: rotate, list
    Delegation(DelegationArgs),
    /// Change your passphrase: re-derives your identity keypair
    Passphrase(PassphraseArgs),
    /// Recover identity via recovery key, or rotate the recovery key
    Recover(RecoverArgs),
}

#[derive(Args)]
struct RecoverArgs {
    /// Recover identity using the recovery key after passphrase loss.
    /// Prompts for the recovery key (or reads it from --recovery), then
    /// asks for a new passphrase and re-wraps seed_wrap_passphrase.
    #[arg(long, conflicts_with = "regenerate_key")]
    recovery_key: bool,

    /// Generate a fresh recovery key for an authenticated session and
    /// re-wrap seed_wrap_recovery only. Requires the current passphrase.
    #[arg(long)]
    regenerate_key: bool,

    /// Recovery key (non-interactive, for scripts and tests).
    #[arg(long)]
    recovery: Option<String>,

    /// New passphrase to set after recovery (non-interactive).
    #[arg(long)]
    new_passphrase: Option<String>,
}

#[derive(Args)]
struct PassphraseArgs {
    /// New passphrase (non-interactive, for scripts and tests). Used in
    /// combination with the global --passphrase flag which supplies the
    /// current passphrase.
    #[arg(long)]
    new_passphrase: Option<String>,
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
}

#[derive(Args)]
struct DelegationArgs {
    #[command(subcommand)]
    command: DelegationCommand,
}

#[derive(Subcommand)]
enum DelegationCommand {
    /// Rotate your delegation keypair for an AI member. Generates a fresh
    /// salt and verifier, invalidates every token you have issued for this
    /// AI, and removes your zone-key wraps for this AI (re-grant where
    /// needed). Other operators' delegations are untouched.
    Rotate(DelegationRotateArgs),
    /// List delegations recorded in project.yaml. Shows every operator's
    /// delegation for the given AI member, or every (operator, AI) pair
    /// when no member id is given.
    Ls(DelegationLsArgs),
}

#[derive(Args)]
struct DelegationRotateArgs {
    /// AI member ID (e.g. ai:claude@joy)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_ai_member))]
    member: String,
}

#[derive(Args)]
struct DelegationLsArgs {
    /// Optional AI member ID (e.g. ai:claude@joy). Without it, every AI
    /// member with at least one operator delegation is listed.
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_ai_member))]
    member: Option<String>,
}

#[derive(Args)]
struct TokenAddArgs {
    /// AI member ID (e.g. ai:claude@joy)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_ai_member))]
    member: String,

    /// Token expiry in hours (default 24; multi-use within the window)
    #[arg(long)]
    ttl: Option<i64>,

    /// Issue with Crypt scope: embed the delegation private key in the
    /// token so the AI can unwrap zone keys for any zone your delegation
    /// has wraps for. Without this flag the token is auth-only.
    #[arg(long)]
    crypt: bool,
}

pub fn run(args: AuthArgs) -> Result<()> {
    match args.command {
        Some(AuthCommand::Init) => run_init(args.passphrase.as_deref(), args.user.as_deref()),
        Some(AuthCommand::Status) => run_status(),
        Some(AuthCommand::Reset(a)) => run_reset(a, args.passphrase.as_deref()),
        Some(AuthCommand::Token(a)) => {
            run_token(a, args.passphrase.as_deref(), args.user.as_deref())
        }
        Some(AuthCommand::Delegation(a)) => match a.command {
            DelegationCommand::Rotate(args_) => {
                run_ai_rotate(&args_.member, args.passphrase.as_deref())
            }
            DelegationCommand::Ls(args_) => run_delegation_ls(args_.member.as_deref()),
        },
        Some(AuthCommand::Passphrase(a)) => {
            run_passphrase(args.passphrase.as_deref(), a.new_passphrase.as_deref())
        }
        Some(AuthCommand::Recover(a)) => run_recover(a, args.passphrase.as_deref()),
        None => {
            if let Some(otp) = args.otp.as_deref() {
                run_auth_otp(otp, args.passphrase.as_deref())
            } else {
                run_auth(
                    args.passphrase.as_deref(),
                    args.token.as_deref(),
                    args.user.as_deref(),
                )
            }
        }
    }
}

/// Resolve the member-selector for this invocation. `--user` always
/// wins; otherwise we fall back to git config user.email. Centralised
/// here so every auth path uses the same rule (JOY-00F3-AE).
fn resolve_user(user_flag: Option<&str>) -> Result<String> {
    match user_flag {
        Some(u) if !u.is_empty() => Ok(u.to_string()),
        _ => Ok(joy_core::vcs::default_vcs().user_email()?),
    }
}

fn auth_state(
    root: &std::path::Path,
) -> Result<(bool, bool, serde_yaml_ng::Value, std::path::PathBuf)> {
    let project_path = store::joy_dir(root).join(store::PROJECT_FILE);
    let security_path = root.join("SECURITY.md");
    let raw = std::fs::read_to_string(&project_path)?;
    let raw_value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&raw)?;
    let (migrated_value, schema_stale) =
        joy_core::migrations::project_yaml::apply(raw_value.clone());
    let security_current = joy_core::security_md::is_current(&security_path)?;
    Ok((
        security_current,
        schema_stale,
        migrated_value,
        security_path,
    ))
}

/// Aggregated check used by `joy update --check`. Prints one line per
/// inspected artefact and returns `true` when anything is stale.
pub(crate) fn run_check_default() -> Result<bool> {
    use crate::color;
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;
    let (security_current, schema_stale, _, _) = auth_state(&root)?;
    let row = |ok: bool, name: &str| {
        let mark = if ok {
            color::check_mark()
        } else {
            color::warn_mark()
        };
        let status = if ok {
            color::inactive("up to date")
        } else {
            color::warning("stale")
        };
        println!("  {mark}{name:<24} {status}");
    };
    row(security_current, "SECURITY.md");
    row(!schema_stale, "project.yaml schema");
    Ok(!security_current || schema_stale)
}

/// `joy update` orchestrator entry point: bring auth-scoped Joy-managed
/// artefacts up to the current Joy version. Renders SECURITY.md from the
/// shipped template (preserving any user content outside the marker
/// block) and normalises `project.yaml` from the legacy auth schema to
/// the current one. Per ADR-035 this is the only place schema migration
/// is persisted.
pub(crate) fn run_update_default() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;
    let (_, schema_stale, migrated_value, security_path) = auth_state(&root)?;
    let project_path = store::joy_dir(&root).join(store::PROJECT_FILE);

    let mut wrote_anything = false;

    let security_changed = joy_core::security_md::render(&security_path)?;
    if security_changed {
        println!("Rendered SECURITY.md at repository root.");
        joy_core::git_ops::auto_git_add(&root, &["SECURITY.md"]);
        wrote_anything = true;
    }

    if schema_stale {
        // Re-deserialize the migrated value into a typed Project and
        // write it back through write_yaml_preserve so any unknown
        // top-level keys (e.g. release config) survive untouched.
        let project: joy_core::model::project::Project = serde_yaml_ng::from_value(migrated_value)?;
        store::write_yaml_preserve(&project_path, &project)?;
        let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
        joy_core::git_ops::auto_git_add(&root, &[&rel]);
        println!("Normalised project.yaml to current auth schema.");
        wrote_anything = true;
    }

    // Warn about AI delegation entries that lack delegation_salt: the
    // operator can still redeem any outstanding token until its TTL
    // expires, but the next `joy auth token add` will refuse to issue
    // a new one until they rotate. Surface the situation here so they
    // are not surprised at issuance time.
    let project = store::read_project(&project_path)?;
    let mut legacy_pairs: Vec<(String, String)> = Vec::new();
    for (operator, member) in &project.members {
        for (ai, entry) in &member.ai_delegations {
            if entry.delegation_salt.is_none() {
                legacy_pairs.push((operator.clone(), ai.clone()));
            }
        }
    }
    if !legacy_pairs.is_empty() {
        println!();
        println!("Legacy AI delegations without a delegation_salt:");
        for (op, ai) in &legacy_pairs {
            println!("  {op} -> {ai}");
        }
        println!(
            "  Existing tokens keep working until their TTL expires; the next \
             `joy auth token add` for these will require `joy auth delegation rotate`."
        );
    }

    if !wrote_anything {
        println!("Already up to date.");
    }
    Ok(())
}

/// Resolve token from --token flag or JOY_TOKEN env var.
fn resolve_token(flag: Option<&str>) -> Option<String> {
    flag.map(|s| s.to_string())
        .or_else(|| std::env::var("JOY_TOKEN").ok().filter(|s| !s.is_empty()))
}

/// Read passphrase from flag or prompt interactively.
///
/// When no flag is given and stdin is not a terminal (piped, redirected
/// from `/dev/null`, or otherwise non-interactive), refuse to prompt and
/// surface a clear error. `rpassword` would otherwise reach for
/// `/dev/tty` directly and block indefinitely on terminals where bats /
/// other test harnesses redirected stdin but cannot intercept the
/// controlling TTY.
///
/// `pub(crate)` so other commands (e.g. `derive_acting_keypair` in the
/// project module) share the same non-interactive-detection rule.
pub(crate) fn read_passphrase(flag: Option<&str>, prompt: &str) -> Result<String> {
    use std::io::IsTerminal;
    match flag {
        Some(p) => Ok(p.to_string()),
        None => {
            if !std::io::stdin().is_terminal() {
                anyhow::bail!(
                    "passphrase required: stdin is not a terminal. \
                     Pass --passphrase <value> for non-interactive use."
                );
            }
            Ok(rpassword::prompt_password(prompt)?)
        }
    }
}

/// `joy auth init` — first-time setup for the current member.
fn run_init(passphrase_flag: Option<&str>, user_flag: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let project_path = store::joy_dir(&root).join(store::PROJECT_FILE);
    let mut project = store::read_project(&project_path)?;

    // Determine who we are
    let email = resolve_user(user_flag)?;
    let member = project.members.get(&email);
    if member.is_none() {
        anyhow::bail!(
            "{} is not a registered project member. Run `joy project member add {}`.",
            email,
            email
        );
    }
    let member = member.unwrap();
    if member.verify_key.is_some() {
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
    validate_passphrase(&passphrase)?;

    // Confirm (only in interactive mode)
    if passphrase_flag.is_none() {
        let confirm = rpassword::prompt_password("  Confirm:    ")?;
        if passphrase != confirm {
            anyhow::bail!("passphrases do not match");
        }
    }

    // Wrapped-seed model (ADR-039): generate a random seed, wrap it under
    // both a passphrase-derived KEK and a recovery-key-derived KEK. The
    // identity keypair derives from the seed and stays stable across
    // passphrase rotation.
    let salt = generate_salt();
    let seed = seed_mod::Seed::generate();
    let recovery = seed_mod::RecoveryKey::generate();
    let wrap_passphrase = seed_mod::wrap_seed_with_passphrase(&seed, &passphrase, &salt)?;
    let wrap_recovery = seed_mod::wrap_seed_with_recovery(&seed, &recovery, &salt)?;
    let keypair = IdentityKeypair::from_seed(seed.as_bytes());
    let public_key = keypair.public_key();

    // Store salt, public key, and both wraps in project.yaml
    let m = project.members.get_mut(&email).unwrap();
    m.kdf_nonce = Some(salt.to_hex());
    m.verify_key = Some(public_key.to_hex());
    m.seed_wrap_passphrase = Some(wrap_passphrase);
    m.seed_wrap_recovery = Some(wrap_recovery);

    // JOY-00FD-93 (also applies to the legacy auth init path): if the
    // founder is the only unattested member, reverse-attest them
    // silently. Closes the attestation chain regardless of the
    // redeemer's capabilities - attestation verification does not
    // require the attester to have manage capability, only that the
    // signature verifies against a member's public_key.
    if let Some(founder_email) = founder_needing_reverse_attestation(&project) {
        if founder_email != email {
            let founder_member = project.members.get(&founder_email).cloned().unwrap();
            let signed_fields = joy_core::auth::attestation::signed_fields_for(
                &founder_email,
                &founder_member.capabilities,
                founder_member.enrollment_verifier.as_deref(),
            );
            let attestation =
                joy_core::auth::attestation::sign_attestation(&email, &keypair, signed_fields);
            project.members.get_mut(&founder_email).unwrap().attestation = Some(attestation);
        }
    }

    store::write_yaml_preserve(&project_path, &project)?;
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    joy_core::git_ops::auto_git_add(&root, &[&rel]);

    // Create initial session
    let project_id = session::project_id(&root)?;
    let session_token = session::create_session(&keypair, &email, &project_id, None);
    session::save_session(&project_id, &session_token)?;

    println!("Authentication initialized for {}.", email);
    println!("Public key registered. Session active (24h).");
    println!();
    println!("RECOVERY KEY (write this down now, it is shown only once):");
    println!();
    println!("    {}", recovery.to_display_string());
    println!();
    println!("Use it with `joy auth recover --recovery-key` if you ever forget");
    println!("your passphrase. Joy never stores the plaintext recovery key.");

    // Render SECURITY.md so first-time setup leaves the repo with the
    // canonical explanation of the public auth fields in place. Idempotent.
    let security_path = root.join("SECURITY.md");
    if joy_core::security_md::render(&security_path)? {
        joy_core::git_ops::auto_git_add(&root, &["SECURITY.md"]);
    }

    joy_core::git_ops::auto_git_post_command(&root, "auth init", &email);

    Ok(())
}

/// `joy auth` — authenticate by passphrase (human) or delegation token (AI).
fn run_auth(
    passphrase_flag: Option<&str>,
    token_flag: Option<&str>,
    user_flag: Option<&str>,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let project = store::load_project(&root)?;
    let project_id = session::project_id(&root)?;

    // Check for delegation token (--token flag or JOY_TOKEN env var)
    if let Some(token_str) = resolve_token(token_flag) {
        return auth_with_token(&root, &project, &project_id, &token_str);
    }

    // Human authentication via passphrase
    let email = resolve_user(user_flag)?;
    auth_with_passphrase(&root, &project, &project_id, &email, passphrase_flag)
}

/// Authenticate a human member via passphrase.
fn auth_with_passphrase(
    root: &std::path::Path,
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

    let public_key_hex = member.verify_key.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "Authentication not initialized for {}. Run `joy auth init`.",
            email
        )
    })?;
    let salt_hex = member
        .kdf_nonce
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No salt found for {}. Run `joy auth init`.", email))?;

    let public_key = PublicKey::from_hex(public_key_hex)?;
    let salt = Salt::from_hex(salt_hex)?;

    let passphrase = read_passphrase(passphrase_flag, "Passphrase: ")?;

    // ADR-039: prefer the wrapped-seed path when seed_wrap_passphrase is
    // present. The legacy path (no wrap) triggers a one-time lazy
    // migration that preserves the existing keypair.
    let keypair = if let Some(wrap_hex) = member.seed_wrap_passphrase.as_deref() {
        let seed = seed_mod::unwrap_seed_with_passphrase(wrap_hex, &passphrase, &salt)?;
        let kp = IdentityKeypair::from_seed(seed.as_bytes());
        if kp.public_key() != public_key {
            anyhow::bail!("incorrect passphrase");
        }
        kp
    } else {
        let key = derive_key(&passphrase, &salt)?;
        let kp = IdentityKeypair::from_derived_key(&key);
        if kp.public_key() != public_key {
            anyhow::bail!("incorrect passphrase");
        }
        // JOY-014C-29 lazy migration: legacy member entry has no
        // seed_wrap_*. Use the derived_key as the seed (it produced
        // the existing verify_key), generate a fresh recovery key,
        // wrap the seed under both KEKs, persist. Identity keypair
        // stays the same; existing Crypt wraps remain valid.
        migrate_to_wrapped_seed(root, email, &key, &salt, &passphrase)?;
        kp
    };

    // JOY-0101-78: silent auto-seal for pre-feature projects. If no
    // member anywhere has an attestation yet, treat the current state as
    // legitimate and sign attestations for every other member using the
    // acting member's fresh keypair. The acting member becomes the trust
    // root and remains unattested until a future joiner reverse-attests
    // them via the normal path. Runs at most once per project, silent.
    let sealed_project = maybe_auto_seal(root, project, email, &keypair)?;
    let project_view: &joy_core::model::project::Project =
        sealed_project.as_ref().unwrap_or(project);
    let member = project_view.members.get(email).unwrap();

    // JOY-0100-DA: verify the member's attestation before establishing a
    // session. Founder may be unattested during the solo phase (trust
    // root); any member without attestation after the first co-member
    // joined is suspect and rejected.
    if let Some(attestation) = member.attestation.as_ref() {
        verify_member_attestation(project_view, email, member, attestation)?;
    } else if founder_must_be_attested(project_view) {
        anyhow::bail!(
            "{} has no attestation and the project has multiple members. \
             The entry appears to have been tampered with. Ask a manage member \
             to remove and re-add {}.",
            email,
            email
        );
    }

    let session_token = session::create_session(&keypair, email, project_id, None);
    session::save_session(project_id, &session_token)?;

    // ADR-040: opportunistic re-lock. We have the seed in hand; walk
    // every zone this member has a wrap for and re-encrypt any
    // plaintext file under crypt.zones[<zone>].paths that the user
    // forgot to lock.
    let relocked = relock_unlocked_files(root, project_view, email, &keypair.to_seed_bytes());
    if relocked > 0 {
        println!("Re-locked {} unlocked file(s).", relocked);
    }

    println!("Authenticated as {}. Session active (24h).", email);

    Ok(())
}

/// Walk `crypt.zones[].paths` for every zone the member is granted to;
/// any file currently in plaintext gets re-encrypted with the
/// matching zone key. Best-effort: errors per file are logged but the
/// walk continues. Returns the number of files re-locked.
fn relock_unlocked_files(
    root: &std::path::Path,
    project: &joy_core::model::project::Project,
    email: &str,
    seed: &[u8; 32],
) -> usize {
    let Some(member) = project.members.get(email) else {
        return 0;
    };
    let mut relocked = 0;
    for (zone, wrap_hex) in &member.crypt_wraps {
        let Ok(zone_key) = joy_core::crypt::unwrap_for_member(wrap_hex, zone, seed) else {
            continue;
        };
        let Some(zone_cfg) = project.crypt.zones.get(zone) else {
            continue;
        };
        for pattern in &zone_cfg.paths {
            relock_path(root, &zone_key, zone, pattern, &mut relocked);
        }
    }
    relocked
}

fn relock_path(
    root: &std::path::Path,
    zone_key: &joy_core::crypt::ZoneKey,
    zone: &str,
    pattern: &str,
    relocked: &mut usize,
) {
    let abs = root.join(pattern);
    if abs.is_file() {
        if relock_file(&abs, zone_key, zone) {
            *relocked += 1;
        }
    } else if abs.is_dir() {
        relock_dir(&abs, zone_key, zone, relocked);
    }
}

fn relock_dir(
    dir: &std::path::Path,
    zone_key: &joy_core::crypt::ZoneKey,
    zone: &str,
    relocked: &mut usize,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            relock_dir(&p, zone_key, zone, relocked);
        } else if p.is_file() && relock_file(&p, zone_key, zone) {
            *relocked += 1;
        }
    }
}

fn relock_file(path: &std::path::Path, zone_key: &joy_core::crypt::ZoneKey, zone: &str) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    if joy_core::crypt::looks_like_blob(&bytes) {
        return false;
    }
    let blob = joy_core::crypt::encrypt_blob(zone, zone_key, &bytes);
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let tmp = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("relock"),
        std::process::id()
    ));
    if std::fs::write(&tmp, &blob).is_err() {
        return false;
    }
    if std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    true
}

/// JOY-014C-29 lazy migration: convert a legacy member entry (no
/// seed_wrap_*) to the wrapped-seed model on first authenticated `joy
/// auth`. Generates a fresh recovery key and writes both wraps. The
/// keypair is preserved because the legacy `derived_key` becomes the
/// new seed; verify_key stays valid.
fn migrate_to_wrapped_seed(
    root: &std::path::Path,
    email: &str,
    derived_key: &joy_core::auth::DerivedKey,
    kdf_nonce: &Salt,
    _passphrase: &str,
) -> Result<()> {
    let project_path = store::joy_dir(root).join(store::PROJECT_FILE);
    let mut project = store::read_project(&project_path)?;

    let seed = seed_mod::Seed::from_derived_key(derived_key);
    let recovery = seed_mod::RecoveryKey::generate();
    // ADR-039: during migration KEK_passphrase happens to equal the
    // seed, so the wrap is structural rather than secret-bearing. After
    // any subsequent passphrase change the wraps decouple naturally.
    let wrap_passphrase = seed_mod::wrap_seed_for_migration(&seed);
    let wrap_recovery = seed_mod::wrap_seed_with_recovery(&seed, &recovery, kdf_nonce)?;

    let m = project
        .members
        .get_mut(email)
        .ok_or_else(|| anyhow::anyhow!("member {} disappeared mid-migration", email))?;
    m.seed_wrap_passphrase = Some(wrap_passphrase);
    m.seed_wrap_recovery = Some(wrap_recovery);

    store::write_yaml_preserve(&project_path, &project)?;
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    joy_core::git_ops::auto_git_add(root, &[&rel]);

    println!();
    println!("Auth schema upgraded to the wrapped-seed identity model.");
    println!("RECOVERY KEY (write this down now, it is shown only once):");
    println!();
    println!("    {}", recovery.to_display_string());
    println!();
    println!("Use it with `joy auth recover --recovery-key` if you ever forget");
    println!("your passphrase. Joy never stores the plaintext recovery key.");
    println!();

    Ok(())
}

/// JOY-0101-78: Silent auto-seal for projects that existed before the
/// attestation feature landed. If no member carries an attestation, sign
/// attestations for every other member using the acting member's keypair,
/// write project.yaml once, and return the sealed state. Otherwise no-op.
///
/// This is a one-shot migration aid; JOY-0105-65 tracks removal of the
/// code path once the deprecation window has passed.
fn maybe_auto_seal(
    root: &std::path::Path,
    project: &joy_core::model::project::Project,
    acting_email: &str,
    acting_keypair: &IdentityKeypair,
) -> Result<Option<joy_core::model::project::Project>> {
    let has_any_attestation = project.members.values().any(|m| m.attestation.is_some());
    if has_any_attestation || project.members.len() < 2 {
        return Ok(None);
    }

    let project_path = store::joy_dir(root).join(store::PROJECT_FILE);
    let mut sealed = store::read_project(&project_path)?;

    let targets: Vec<String> = sealed
        .members
        .keys()
        .filter(|email| email.as_str() != acting_email)
        .cloned()
        .collect();
    for target_email in targets {
        let target = sealed.members.get(&target_email).cloned().unwrap();
        let signed_fields = joy_core::auth::attestation::signed_fields_for(
            &target_email,
            &target.capabilities,
            target.enrollment_verifier.as_deref(),
        );
        let attestation = joy_core::auth::attestation::sign_attestation(
            acting_email,
            acting_keypair,
            signed_fields,
        );
        sealed.members.get_mut(&target_email).unwrap().attestation = Some(attestation);
    }

    store::write_yaml_preserve(&project_path, &sealed)?;
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    joy_core::git_ops::auto_git_add(root, &[&rel]);

    Ok(Some(sealed))
}

/// Verify the attestation against the attester's public_key in project.yaml
/// and against the member's current fields. Produces user-facing error
/// messages aligned with the CLI UX (not bare JoyError strings).
fn verify_member_attestation(
    project: &joy_core::model::project::Project,
    email: &str,
    member: &joy_core::model::project::Member,
    attestation: &joy_core::model::project::Attestation,
) -> Result<()> {
    let attester_entry = project.members.get(&attestation.attester).ok_or_else(|| {
        anyhow::anyhow!(
            "attestation for {} names attester {} but that member is not registered. \
             Ask a manage member to remove and re-add {}.",
            email,
            attestation.attester,
            email
        )
    })?;
    let attester_pubkey_hex = attester_entry.verify_key.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "attestation for {} is signed by {} but that member has no public key. \
             Ask a manage member to remove and re-add {}.",
            email,
            attestation.attester,
            email
        )
    })?;
    let attester_pubkey = PublicKey::from_hex(attester_pubkey_hex)?;
    joy_core::auth::attestation::verify_attestation(attestation, &attester_pubkey, email, member)
        .map_err(|e| {
            anyhow::anyhow!(
                "attestation for {} is not valid ({}). \
                 The entry appears to have been tampered with. \
                 Ask a manage member to remove and re-add {}.",
                email,
                e,
                email
            )
        })
}

/// An unattested member is legitimate only as the sole trust root in a
/// project that has not yet completed the reverse-attestation closure.
/// Once any pair of members mutually attests each other (A attested B,
/// B attested A), the chain has closed and every member - including any
/// former trust root - must carry an attestation. A lone unattested
/// entry that appears after closure is tampering.
fn founder_must_be_attested(project: &joy_core::model::project::Project) -> bool {
    let unattested = project
        .members
        .values()
        .filter(|m| m.attestation.is_none())
        .count();
    if unattested > 1 {
        return true;
    }
    // If a mutual-attestation pair exists, the reverse-attestation step
    // has happened; no unattested member is permitted anymore.
    if unattested == 1 && has_mutual_attestation_pair(project) {
        return true;
    }
    false
}

fn has_mutual_attestation_pair(project: &joy_core::model::project::Project) -> bool {
    for (email, member) in &project.members {
        let Some(att) = &member.attestation else {
            continue;
        };
        if let Some(attester) = project.members.get(&att.attester) {
            if let Some(attester_att) = &attester.attestation {
                if attester_att.attester == *email {
                    return true;
                }
            }
        }
    }
    false
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
    let human_pk_hex = human_member.verify_key.as_ref().ok_or_else(|| {
        anyhow::anyhow!("Delegating member {} has no public key registered.", human)
    })?;
    let human_pk = PublicKey::from_hex(human_pk_hex)?;

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
    let delegation_pk = PublicKey::from_hex(&delegation_entry.delegation_verifier)?;

    // Validate dual signatures + project + expiry. Tokens are multi-use
    // within their TTL (ADR-034 relaxes ADR-033 §3): no consumed-tokens
    // ledger, redemption of the same token from multiple shells or at
    // multiple points in time produces independent sessions.
    let claims = token::validate_token(&delegation, &human_pk, &delegation_pk, project_id)?;

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
    let ephemeral_keypair = IdentityKeypair::from_random();
    let ephemeral_private = ephemeral_keypair.to_seed_bytes();

    // ADR-041 §5: when the token carries the `crypt` scope, the embedded
    // delegation private key (32-byte Ed25519 seed) is propagated through
    // JOY_SESSION so subsequent joy commands on this AI session can unwrap
    // zone keys without further passphrase entry by the operator.
    let delegation_private: Option<[u8; 32]> = if claims.has_crypt_scope() {
        match delegation.delegation_private_key.as_ref() {
            Some(hex_seed) => {
                let bytes = hex::decode(hex_seed).map_err(|e| {
                    anyhow::anyhow!("Token has malformed delegation_private_key: {e}")
                })?;
                if bytes.len() != 32 {
                    anyhow::bail!(
                        "Token's delegation_private_key has wrong length: expected 32, got {}",
                        bytes.len()
                    );
                }
                let mut seed = [0u8; 32];
                seed.copy_from_slice(&bytes);
                // Verify the seed produces the public key we expect.
                let derived = IdentityKeypair::from_seed(&seed);
                if derived.public_key().to_hex() != delegation_entry.delegation_verifier {
                    anyhow::bail!(
                        "Token's delegation_private_key does not match the registered \
                         delegation_verifier. The delegation may have been rotated since the \
                         token was issued; ask the operator for a fresh token."
                    );
                }
                Some(seed)
            }
            None => anyhow::bail!(
                "Token claims include the `crypt` scope but the delegation private key is \
                 missing from the token. Ask the operator to re-issue with --crypt."
            ),
        }
    } else {
        None
    };

    // ADR-041 §6: bound the session lifetime by the token expiry so a
    // short-lived Crypt token (e.g. --ttl 30m) actually grants only that
    // window of access.
    let session_token = session::create_session_for_ai(
        &ephemeral_keypair,
        &claims.ai_member,
        project_id,
        None,
        &delegation_entry.delegation_verifier,
        claims.expires,
    );
    session::save_session(project_id, &session_token)?;

    // Output session handle for eval (stdout) -- SSH-agent pattern.
    // Status message goes to stderr so `eval $(joy auth --token ...)` works.
    // JOY_SESSION carries the ephemeral private key (ADR-033 §2). It is
    // intentionally not persisted to any tool config file: disk persistence
    // would contradict the proof-of-possession property. The AI tool is
    // responsible for propagating the env value into its subshells.
    let sid = session::session_id(project_id, &claims.ai_member);
    let env_value =
        session::encode_session_env_full(&sid, &ephemeral_private, delegation_private.as_ref());

    if crate::output::is_json() {
        #[derive(serde::Serialize)]
        struct TokenAuthPayload<'a> {
            session_env: String,
            member: &'a str,
            delegated_by: &'a str,
            project_id: &'a str,
        }
        crate::output::emit(TokenAuthPayload {
            session_env: env_value.clone(),
            member: &claims.ai_member,
            delegated_by: &claims.delegated_by,
            project_id,
        })?;
    } else {
        println!("export JOY_SESSION={env_value}");
        eprintln!(
            "Authenticated as {} (delegated by {}). Session active (24h).",
            claims.ai_member, claims.delegated_by
        );
    }

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

/// `joy auth status` — show current session state and any AI sessions
/// the calling user has delegated to.
fn run_status() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let identity =
        joy_core::identity::resolve_identity(&root).map_err(|e| anyhow::anyhow!("{e}"))?;
    let project = store::load_project(&root)?;
    let project_id = session::project_id(&root)?;

    let own_session = if identity.authenticated {
        session::load_session(&project_id, &identity.member)
            .ok()
            .flatten()
    } else {
        None
    };
    let expires_in_seconds = own_session
        .as_ref()
        .map(|s| (s.claims.expires - Utc::now()).num_seconds());
    let auth_initialized = project
        .members
        .get(&identity.member)
        .is_some_and(|m| m.verify_key.is_some());

    // Delegated AI sessions: only the human delegator's own ai_delegations
    // are surfaced here. We do not enumerate sessions delegated by other
    // humans -- that is not this caller's audit surface.
    let delegated_sessions: Vec<DelegatedSession> = project
        .members
        .get(&identity.member)
        .map(|m| m.ai_delegations.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|ai_id| {
            let sess = session::load_session(&project_id, &ai_id).ok().flatten();
            let active = sess.as_ref().is_some_and(|s| s.claims.expires > Utc::now());
            let expires_in_seconds = sess
                .as_ref()
                .filter(|_| active)
                .map(|s| (s.claims.expires - Utc::now()).num_seconds());
            DelegatedSession {
                member: ai_id,
                active,
                expires_in_seconds,
            }
        })
        .collect();

    if crate::output::is_json() {
        crate::output::emit(AuthStatusPayload {
            authenticated: identity.authenticated,
            member: identity.member.clone(),
            delegated_by: identity.delegated_by.clone(),
            session_present: own_session.is_some(),
            expires_in_seconds,
            auth_initialized,
            delegated_sessions,
        })?;
        if !identity.authenticated {
            std::process::exit(1);
        }
        return Ok(());
    }

    let w = color::terminal_width();
    println!("{}", color::header("Auth Status"));

    println!("{}", color::section("Your session"));
    if identity.authenticated {
        if let Some(sess) = &own_session {
            let remaining = sess.claims.expires - Utc::now();
            println!(
                "  {} {}",
                color::label("Member:    "),
                color::id(&identity.member)
            );
            if let Some(ref by) = identity.delegated_by {
                println!("  {} {}", color::label("Delegated: "), by);
            }
            println!(
                "  {} {}h {}m",
                color::label("Expires:   "),
                remaining.num_hours(),
                remaining.num_minutes() % 60
            );
        } else {
            println!(
                "  {}",
                color::warning(&format!(
                    "Authenticated as {} (session file missing).",
                    identity.member
                ))
            );
        }
    } else if auth_initialized {
        println!(
            "  {}",
            color::inactive(&format!(
                "No active session for {}. Run `joy auth` to authenticate.",
                identity.member
            ))
        );
    } else {
        println!(
            "  {}",
            color::inactive(&format!(
                "Authentication not initialized for {}. Run `joy auth init`.",
                identity.member
            ))
        );
    }

    if !delegated_sessions.is_empty() {
        println!();
        println!("{}", color::section("Delegated AI sessions"));
        for d in &delegated_sessions {
            if d.active {
                let secs = d.expires_in_seconds.unwrap_or(0);
                let hours = secs / 3600;
                let minutes = (secs / 60) % 60;
                println!(
                    "  {} {} {}h {}m",
                    color::id(&d.member),
                    color::check_mark(),
                    hours,
                    minutes
                );
            } else {
                println!(
                    "  {} {}",
                    color::id(&d.member),
                    color::inactive("no active session")
                );
            }
        }
    }

    println!("{}", color::label(&"-".repeat(w)));

    if !identity.authenticated {
        std::process::exit(1);
    }

    Ok(())
}

#[derive(serde::Serialize)]
struct AuthStatusPayload {
    authenticated: bool,
    member: String,
    delegated_by: Option<String>,
    session_present: bool,
    expires_in_seconds: Option<i64>,
    auth_initialized: bool,
    delegated_sessions: Vec<DelegatedSession>,
}

#[derive(serde::Serialize)]
struct DelegatedSession {
    member: String,
    active: bool,
    expires_in_seconds: Option<i64>,
}

/// `joy auth reset [member]` — reset authentication for yourself or another member.
fn run_reset(args: ResetArgs, passphrase_flag: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let project_path = store::joy_dir(&root).join(store::PROJECT_FILE);
    let mut project = store::read_project(&project_path)?;
    let email = joy_core::vcs::default_vcs().user_email()?;

    let target = args.member.as_deref().unwrap_or(&email);
    let resetting_other = target != email;

    // Verify the acting user's identity via passphrase
    let acting_member = project
        .members
        .get(&email)
        .ok_or_else(|| anyhow::anyhow!("{} is not a registered project member.", email))?;

    if acting_member.verify_key.is_none() {
        anyhow::bail!(
            "Authentication not initialized for {}. Run `joy auth init`.",
            email
        );
    }

    // Authenticate the acting user
    let passphrase = read_passphrase(passphrase_flag, "Passphrase: ")?;
    let _ = joy_core::auth::unlock_identity(acting_member, &passphrase)?;

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
    m.verify_key = None;
    m.kdf_nonce = None;
    m.seed_wrap_passphrase = None;
    m.seed_wrap_recovery = None;
    m.enrollment_verifier = None;

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
fn run_token(
    args: TokenArgs,
    passphrase_flag: Option<&str>,
    user_flag: Option<&str>,
) -> Result<()> {
    match args.command {
        TokenCommand::Add(a) => run_token_add(a, passphrase_flag, user_flag),
    }
}

/// `joy auth token add <ai-member>` — create a delegation token.
fn run_token_add(
    args: TokenAddArgs,
    passphrase_flag: Option<&str>,
    user_flag: Option<&str>,
) -> Result<()> {
    use joy_core::model::project::is_ai_member;

    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let project = store::load_project(&root)?;
    let email = resolve_user(user_flag)?;

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

    // Authenticate the acting human first. We do this before the guard
    // check so that a cold-start (no active session) does not require
    // running `joy auth` separately: the passphrase entered here covers
    // both signing the delegation and bootstrapping the session
    // (JOY-00EF-E5).
    let member = project
        .members
        .get(&email)
        .ok_or_else(|| anyhow::anyhow!("{} is not a registered project member.", email))?;
    if member.verify_key.is_none() {
        anyhow::bail!(
            "Authentication not initialized for {}. Run `joy auth init`.",
            email
        );
    }

    let passphrase = read_passphrase(passphrase_flag, "Passphrase: ")?;
    let unlocked = joy_core::auth::unlock_identity(member, &passphrase)?;
    let keypair = unlocked.keypair;
    let identity_seed = unlocked.seed;

    // If no session exists, create one from the keypair we just derived.
    // The guard check below then succeeds in the same invocation, so the
    // user does not have to run `joy auth` separately.
    let project_id = session::project_id(&root)?;
    if session::load_session(&project_id, &email)?.is_none() {
        let session_token = session::create_session(&keypair, &email, &project_id, None);
        session::save_session(&project_id, &session_token)?;
    }

    // Guard: requires manage capability. We construct an explicit
    // Identity from the resolved member id so that --user takes effect
    // here too (otherwise resolve_identity would fall back to the git
    // email and refuse the action -- JOY-00F3-AE).
    let identity = joy_core::identity::Identity {
        member: email.clone(),
        delegated_by: None,
        authenticated: true,
    };
    joy_core::guard::Guard::load(&root)?
        .check(&joy_core::guard::Action::ManageProject, &identity)
        .enforce(&root, "project", &identity)?;

    // The delegation private key is never persisted on disk. Three cases:
    //   - no project.yaml entry yet  -> bootstrap: fresh salt, derive
    //     seed, register verifier+salt in one project.yaml write.
    //   - entry has delegation_salt -> re-derive seed from the operator's
    //     passphrase-unwrapped identity seed plus the recorded salt; no
    //     project.yaml write. Verifier double-checked.
    //   - entry has no delegation_salt (legacy random keypair) -> bail
    //     with a rotate-first message; the original seed is unrecoverable.
    let existing_entry = member.ai_delegations.get(&args.member);
    let existing_public = existing_entry.map(|e| e.delegation_verifier.clone());
    let existing_salt = existing_entry.and_then(|e| e.delegation_salt.clone());

    // `delegation_salt_to_persist` is `Some` only when we end up writing
    // a brand-new project.yaml entry (the bootstrap case). The 32-byte
    // seed comes back so the caller can embed it in `--crypt` tokens
    // (ADR-041 §3).
    let (delegation_keypair, delegation_seed, delegation_salt_to_persist): (
        IdentityKeypair,
        [u8; 32],
        Option<String>,
    ) = match (&existing_public, &existing_salt) {
        (None, _) => {
            let new_salt = generate_salt();
            let seed = delegation::derive_delegation_seed(
                &identity_seed,
                &new_salt,
                &project_id,
                &args.member,
            );
            let kp = IdentityKeypair::from_seed(&seed);
            (kp, seed, Some(new_salt.to_hex()))
        }
        (Some(pub_hex), Some(salt_hex)) => {
            let salt = Salt::from_hex(salt_hex)?;
            let seed = delegation::derive_delegation_seed(
                &identity_seed,
                &salt,
                &project_id,
                &args.member,
            );
            let kp = IdentityKeypair::from_seed(&seed);
            if kp.public_key().to_hex() != *pub_hex {
                anyhow::bail!(
                    "Re-derived delegation key for {m} does not match the public key \
                     recorded in project.yaml. The salt or your identity may have been \
                     rotated since this checkout was last updated. Pull the latest \
                     project.yaml and retry, or run `joy auth delegation rotate {m}` to \
                     start a fresh delegation.",
                    m = args.member
                );
            }
            (kp, seed, None)
        }
        (Some(_), None) => {
            anyhow::bail!(
                "Cannot issue a new token for {m}.\n  \
                 Run:  joy auth delegation rotate {m}\n  \
                 Existing tokens keep working until their TTL expires.",
                m = args.member
            )
        }
    };

    let new_entry = delegation_salt_to_persist.is_some();
    let delegation_salt_hex = delegation_salt_to_persist;

    // ADR-034 relaxes §3: tokens are multi-use within a single TTL. Default
    // 24h covers a typical working session; human can override with --ttl.
    const DEFAULT_TOKEN_TTL_HOURS: i64 = 24;
    let ttl = Some(
        args.ttl
            .map(chrono::Duration::hours)
            .unwrap_or_else(|| chrono::Duration::hours(DEFAULT_TOKEN_TTL_HOURS)),
    );
    let token_obj = token::create_token(
        token::TokenSigningKeys {
            delegator: &keypair,
            delegation: &delegation_keypair,
            delegation_seed: &delegation_seed,
        },
        token::TokenIssueParams {
            ai_member: &args.member,
            human: &email,
            project_id: &project_id,
            ttl,
            crypt_scope: args.crypt,
        },
    );

    // Persist the delegation public key on first issuance. Subsequent
    // issuances for the same (human, AI) pair produce no project.yaml
    // write since the key is stable (ADR-033).
    let project_path = store::joy_dir(&root).join(store::PROJECT_FILE);
    let mut project_mut = store::read_project(&project_path)?;
    if new_entry {
        if let Some(m) = project_mut.members.get_mut(&email) {
            m.ai_delegations.insert(
                args.member.clone(),
                joy_core::model::project::AiDelegationEntry {
                    delegation_verifier: delegation_keypair.public_key().to_hex(),
                    delegation_salt: delegation_salt_hex.clone(),
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

    if crate::output::is_json() {
        #[derive(serde::Serialize)]
        struct TokenAddPayload<'a> {
            member: &'a str,
            token: String,
            ttl_hours: i64,
        }
        return crate::output::emit(TokenAddPayload {
            member: &args.member,
            token: encoded,
            ttl_hours: hours,
        });
    }

    println!("Delegation token for {}:", args.member);
    println!();
    println!("  {}", encoded);
    println!();
    println!("The AI redeems it with:");
    println!("  joy auth --token {}", encoded);
    println!();
    println!(
        "Token expires in {hours} hours. It may be redeemed multiple times within that window."
    );

    Ok(())
}

/// `joy auth delegation ls [ai-member]` -- list registered delegations.
///
/// Reads `project.yaml`'s `members.*.ai_delegations.*` and prints one
/// row per registered (operator, AI) pair. With a member id, the output
/// is filtered to that AI; without, every (operator, AI) pair is shown.
fn run_delegation_ls(filter_member: Option<&str>) -> Result<()> {
    use joy_core::model::project::is_ai_member;
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;
    let project = store::load_project(&root)?;

    if let Some(m) = filter_member {
        if !is_ai_member(m) {
            anyhow::bail!("{m} is not an AI member (must start with ai:)");
        }
    }

    #[derive(serde::Serialize)]
    struct Row<'a> {
        operator: &'a str,
        ai_member: &'a str,
        delegation_verifier: &'a str,
        created: String,
        rotated: Option<String>,
    }

    let mut rows: Vec<Row<'_>> = Vec::new();
    for (operator, member) in &project.members {
        for (ai, entry) in &member.ai_delegations {
            if let Some(filter) = filter_member {
                if filter != ai {
                    continue;
                }
            }
            rows.push(Row {
                operator,
                ai_member: ai,
                delegation_verifier: &entry.delegation_verifier,
                created: entry.created.format("%Y-%m-%d %H:%M UTC").to_string(),
                rotated: entry
                    .rotated
                    .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string()),
            });
        }
    }

    if crate::output::is_json() {
        return crate::output::emit(rows);
    }

    if rows.is_empty() {
        match filter_member {
            Some(m) => println!("No delegations registered for {m}."),
            None => println!("No AI delegations registered."),
        }
        return Ok(());
    }

    let op_w = rows.iter().map(|r| r.operator.len()).max().unwrap_or(8);
    let ai_w = rows.iter().map(|r| r.ai_member.len()).max().unwrap_or(8);
    println!(
        "{:<op_w$}  {:<ai_w$}  {:<22}  ROTATED",
        "OPERATOR",
        "AI MEMBER",
        "CREATED",
        op_w = op_w,
        ai_w = ai_w
    );
    for r in &rows {
        println!(
            "{:<op_w$}  {:<ai_w$}  {:<22}  {}",
            r.operator,
            r.ai_member,
            r.created,
            r.rotated.as_deref().unwrap_or("-"),
            op_w = op_w,
            ai_w = ai_w
        );
    }
    Ok(())
}

/// `joy auth passphrase` - change the current member's passphrase.
///
/// Verifies the current passphrase against the stored public key,
/// derives a fresh keypair from the new passphrase + a fresh salt,
/// writes the new public_key and salt, and invalidates any active
/// session for this member. Attestations on this member remain valid
/// because `public_key` is not in the signed_fields set (JOY-00FB-58).
fn run_passphrase(current_flag: Option<&str>, new_flag: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let project_path = store::joy_dir(&root).join(store::PROJECT_FILE);
    let mut project = store::read_project(&project_path)?;

    let email = joy_core::vcs::default_vcs().user_email()?;
    let member = project
        .members
        .get(&email)
        .ok_or_else(|| anyhow::anyhow!("{} is not a registered project member", email))?;
    let current_pub_hex = member.verify_key.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "Authentication not initialized for {}. Run `joy auth init`.",
            email
        )
    })?;
    let current_salt_hex = member
        .kdf_nonce
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No salt registered for {}.", email))?;
    let current_pub = PublicKey::from_hex(current_pub_hex)?;
    let current_salt = Salt::from_hex(current_salt_hex)?;

    let current_pass = read_passphrase(current_flag, "Current passphrase: ")?;

    // ADR-039: in the wrapped-seed model the seed and keypair are stable
    // across passphrase rotation; we only re-wrap seed_wrap_passphrase.
    // Legacy entries (no seed_wrap_*) are migrated transparently first
    // and then re-wrapped under the new passphrase.
    let seed = if let Some(wrap_hex) = member.seed_wrap_passphrase.as_deref() {
        seed_mod::unwrap_seed_with_passphrase(wrap_hex, &current_pass, &current_salt)?
    } else {
        let current_key = derive_key(&current_pass, &current_salt)?;
        let current_kp = IdentityKeypair::from_derived_key(&current_key);
        if current_kp.public_key() != current_pub {
            anyhow::bail!("incorrect passphrase");
        }
        // Same migration path as auth_with_passphrase: derived_key
        // becomes the seed.
        let migrated_seed = seed_mod::Seed::from_derived_key(&current_key);
        let recovery = seed_mod::RecoveryKey::generate();
        let m = project.members.get_mut(&email).unwrap();
        m.seed_wrap_passphrase = Some(seed_mod::wrap_seed_for_migration(&migrated_seed));
        m.seed_wrap_recovery = Some(seed_mod::wrap_seed_with_recovery(
            &migrated_seed,
            &recovery,
            &current_salt,
        )?);
        // Recovery key must reach the user; print before continuing so a
        // crash mid-rewrap leaves the recovery path intact.
        println!();
        println!("Auth schema upgraded to the wrapped-seed identity model.");
        println!("RECOVERY KEY (write this down now, it is shown only once):");
        println!();
        println!("    {}", recovery.to_display_string());
        println!();
        migrated_seed
    };

    let new_pass = read_passphrase(new_flag, "New passphrase:     ")?;
    if new_pass == current_pass {
        anyhow::bail!("new passphrase must differ from the current one");
    }
    validate_passphrase(&new_pass)?;
    if new_flag.is_none() {
        let confirm = rpassword::prompt_password("Confirm:            ")?;
        if confirm != new_pass {
            anyhow::bail!("passphrases do not match");
        }
    }

    // Re-wrap the seed under the new passphrase KEK. kdf_nonce stays
    // stable so seed_wrap_recovery remains valid (recovery key is not
    // rotated by this command). verify_key does not change because the
    // keypair derives from the unchanged seed.
    let new_wrap_passphrase = seed_mod::wrap_seed_with_passphrase(&seed, &new_pass, &current_salt)?;

    let m = project.members.get_mut(&email).unwrap();
    m.seed_wrap_passphrase = Some(new_wrap_passphrase);
    store::write_yaml_preserve(&project_path, &project)?;
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    joy_core::git_ops::auto_git_add(&root, &[&rel]);

    let project_id = session::project_id(&root)?;
    let _ = session::remove_session(&project_id, &email);

    println!("Passphrase changed for {}.", email);
    println!("Prior sessions are invalidated. Run `joy auth` to start a fresh session.");

    joy_core::git_ops::auto_git_post_command(&root, "auth passphrase", &email);

    Ok(())
}

/// `joy auth recover` - recovery-key paths (ADR-039).
///
/// `--recovery-key`: passphrase-loss recovery. User provides their
/// recovery key plus a new passphrase. Joy unwraps the seed via the
/// recovery KEK, re-wraps it under the new passphrase KEK, leaves the
/// recovery wrap untouched. Identity keypair is preserved.
///
/// `--regenerate-key`: rotate the recovery key. User authenticates with
/// the current passphrase. Joy unwraps the seed, generates a new
/// recovery key, re-wraps the seed under the new recovery KEK, leaves
/// the passphrase wrap untouched. Old recovery key becomes useless.
fn run_recover(args: RecoverArgs, passphrase_flag: Option<&str>) -> Result<()> {
    if !args.recovery_key && !args.regenerate_key {
        anyhow::bail!(
            "specify --recovery-key (passphrase loss) or --regenerate-key (rotate recovery key)"
        );
    }

    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;
    let project_path = store::joy_dir(&root).join(store::PROJECT_FILE);
    let mut project = store::read_project(&project_path)?;

    let email = joy_core::vcs::default_vcs().user_email()?;
    let member = project
        .members
        .get(&email)
        .ok_or_else(|| anyhow::anyhow!("{} is not a registered project member", email))?;
    let salt_hex = member
        .kdf_nonce
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Authentication not initialized for {}.", email))?;
    let salt = Salt::from_hex(salt_hex)?;

    if args.recovery_key {
        let wrap_recovery_hex = member.seed_wrap_recovery.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "{} has no recovery wrap. The legacy auth schema needs `joy auth` once first \
                 to migrate, which also reveals the recovery key.",
                email
            )
        })?;

        let recovery_str = match args.recovery.as_deref() {
            Some(s) => s.to_string(),
            None => rpassword::prompt_password("Recovery key: ")?,
        };
        let recovery = seed_mod::RecoveryKey::from_user_input(&recovery_str)?;
        let seed = seed_mod::unwrap_seed_with_recovery(wrap_recovery_hex, &recovery, &salt)?;

        let new_pass = read_passphrase(args.new_passphrase.as_deref(), "New passphrase: ")?;
        validate_passphrase(&new_pass)?;
        if args.new_passphrase.is_none() {
            let confirm = rpassword::prompt_password("Confirm:        ")?;
            if confirm != new_pass {
                anyhow::bail!("passphrases do not match");
            }
        }

        let new_wrap_passphrase = seed_mod::wrap_seed_with_passphrase(&seed, &new_pass, &salt)?;
        let m = project.members.get_mut(&email).unwrap();
        m.seed_wrap_passphrase = Some(new_wrap_passphrase);
        store::write_yaml_preserve(&project_path, &project)?;
        let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
        joy_core::git_ops::auto_git_add(&root, &[&rel]);

        let project_id = session::project_id(&root)?;
        let _ = session::remove_session(&project_id, &email);

        println!("Recovery successful. Passphrase reset for {}.", email);
        println!(
            "Run `joy auth` with the new passphrase to start a session. The recovery key remains valid."
        );
        joy_core::git_ops::auto_git_post_command(&root, "auth recover --recovery-key", &email);
    } else {
        // --regenerate-key: rotate the recovery wrap.
        let wrap_passphrase_hex = member.seed_wrap_passphrase.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "{} has no passphrase wrap. The legacy auth schema needs `joy auth` once first.",
                email
            )
        })?;

        let passphrase = read_passphrase(passphrase_flag, "Passphrase: ")?;
        let seed = seed_mod::unwrap_seed_with_passphrase(wrap_passphrase_hex, &passphrase, &salt)?;

        let new_recovery = seed_mod::RecoveryKey::generate();
        let new_wrap_recovery = seed_mod::wrap_seed_with_recovery(&seed, &new_recovery, &salt)?;
        let m = project.members.get_mut(&email).unwrap();
        m.seed_wrap_recovery = Some(new_wrap_recovery);
        store::write_yaml_preserve(&project_path, &project)?;
        let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
        joy_core::git_ops::auto_git_add(&root, &[&rel]);

        println!("Recovery key rotated for {}.", email);
        println!();
        println!("NEW RECOVERY KEY (write this down now, it is shown only once):");
        println!();
        println!("    {}", new_recovery.to_display_string());
        println!();
        println!("The previous recovery key is now invalid.");
        joy_core::git_ops::auto_git_post_command(&root, "auth recover --regenerate-key", &email);
    }

    Ok(())
}

/// `joy auth --otp <code> --passphrase <new>` - redeem a one-time password
/// and set the member's passphrase (JOY-0072). First-time onboarding for
/// a newly-added human member.
///
/// If the redeeming member has manage capability and the founder entry
/// currently has no attestation, reverse-attests the founder with the
/// redeemer's fresh identity key (JOY-00FD-93). Closes the attestation
/// chain implicitly, without CLI output.
fn run_auth_otp(otp: &str, passphrase_flag: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let project_path = store::joy_dir(&root).join(store::PROJECT_FILE);
    let mut project = store::read_project(&project_path)?;

    let email = joy_core::vcs::default_vcs().user_email()?;
    let member = project.members.get(&email).ok_or_else(|| {
        anyhow::anyhow!(
            "{} is not a registered project member. A manage member must add you first.",
            email
        )
    })?;

    let stored_hash = member.enrollment_verifier.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "no pending OTP for {}. Either this member has already completed setup \
             or was added without an OTP.",
            email
        )
    })?;

    if !joy_core::auth::otp::verify_otp(otp, stored_hash)? {
        anyhow::bail!("incorrect OTP");
    }
    // Wrapped-seed onboarding (ADR-039): generate a random seed and
    // recovery key, wrap the seed under both KEKs.
    let passphrase = read_passphrase(passphrase_flag, "Choose passphrase: ")?;
    validate_passphrase(&passphrase)?;
    let salt = generate_salt();
    let seed = seed_mod::Seed::generate();
    let recovery = seed_mod::RecoveryKey::generate();
    let wrap_passphrase = seed_mod::wrap_seed_with_passphrase(&seed, &passphrase, &salt)?;
    let wrap_recovery = seed_mod::wrap_seed_with_recovery(&seed, &recovery, &salt)?;
    let keypair = IdentityKeypair::from_seed(seed.as_bytes());

    // Apply to project.yaml: set public_key/salt/wraps, clear otp_hash.
    {
        let m = project.members.get_mut(&email).unwrap();
        m.verify_key = Some(keypair.public_key().to_hex());
        m.kdf_nonce = Some(salt.to_hex());
        m.seed_wrap_passphrase = Some(wrap_passphrase);
        m.seed_wrap_recovery = Some(wrap_recovery);
        m.enrollment_verifier = None;
    }

    // JOY-00FD-93: if the founder is still the only unattested member,
    // reverse-attest them silently. Attestation verification doesn't
    // require the attester to have manage capability, only that their
    // public_key verifies the signature - so any redeemer (regardless
    // of capabilities) can close the attestation chain on first join.
    if let Some(founder_email) = founder_needing_reverse_attestation(&project) {
        if founder_email != email {
            let founder_member = project.members.get(&founder_email).cloned().unwrap();
            let signed_fields = joy_core::auth::attestation::signed_fields_for(
                &founder_email,
                &founder_member.capabilities,
                founder_member.enrollment_verifier.as_deref(),
            );
            let attestation =
                joy_core::auth::attestation::sign_attestation(&email, &keypair, signed_fields);
            project.members.get_mut(&founder_email).unwrap().attestation = Some(attestation);
        }
    }

    store::write_yaml_preserve(&project_path, &project)?;
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    joy_core::git_ops::auto_git_add(&root, &[&rel]);

    // Establish an initial session for the new member.
    let project_id = session::project_id(&root)?;
    let session_token = session::create_session(&keypair, &email, &project_id, None);
    session::save_session(&project_id, &session_token)?;

    println!("Authentication initialized for {}.", email);
    println!("Public key registered. Session active (24h).");
    println!();
    println!("RECOVERY KEY (write this down now, it is shown only once):");
    println!();
    println!("    {}", recovery.to_display_string());
    println!();
    println!("Use it with `joy auth recover --recovery-key` if you ever forget");
    println!("your passphrase. Joy never stores the plaintext recovery key.");

    joy_core::git_ops::auto_git_post_command(&root, "auth otp", &email);

    Ok(())
}

/// Return the founder's email if exactly one member currently has no
/// attestation (the solo founder, pre-closure). `None` otherwise.
fn founder_needing_reverse_attestation(
    project: &joy_core::model::project::Project,
) -> Option<String> {
    let mut unattested: Vec<&String> = project
        .members
        .iter()
        .filter(|(_, m)| m.attestation.is_none())
        .map(|(email, _)| email)
        .collect();
    if unattested.len() == 1 {
        Some(unattested.remove(0).clone())
    } else {
        None
    }
}

/// `joy ai rotate <ai-member>` - rotate the (human, AI) delegation keypair (ADR-033).
///
/// Replaces the delegation keypair for the acting human and the given AI
/// member with a fresh one: generates a new Ed25519 pair, writes the
/// private half to local state (overwriting any prior file), updates
/// `project.yaml` with the new public key and a `rotated` timestamp in a
/// single commit. Any tokens signed by the prior keypair, and any sessions
/// bound to those tokens, become invalid (signature verification against
/// the new delegation public key will fail).
///
/// Precondition: a delegation entry exists in `project.yaml` for
/// `(acting human, member)`. For the initial delegation use
/// `joy auth token add`, not rotate.
pub fn run_ai_rotate(member: &str, passphrase_flag: Option<&str>) -> Result<()> {
    use joy_core::model::project::is_ai_member;

    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let project = store::load_project(&root)?;
    let email = joy_core::vcs::default_vcs().user_email()?;

    if !is_ai_member(member) {
        anyhow::bail!("{} is not an AI member (must start with ai:)", member);
    }
    if !project.members.contains_key(member) {
        anyhow::bail!("{} is not a registered project member.", member);
    }

    joy_core::guard::enforce(&root, &joy_core::guard::Action::ManageProject, "project")?;

    let human = project
        .members
        .get(&email)
        .ok_or_else(|| anyhow::anyhow!("{} is not a registered project member.", email))?;
    if human.verify_key.is_none() {
        anyhow::bail!(
            "Authentication not initialized for {}. Run `joy auth init`.",
            email
        );
    }

    // Rotation requires an existing delegation. Bootstrap (first-time
    // setup) goes through `joy auth token add`, where project.yaml writes
    // happen lazily on the very first issuance.
    if !human.ai_delegations.contains_key(member) {
        anyhow::bail!(
            "No delegation for {m} is recorded in project.yaml under {email}. \
             Rotation replaces an existing keypair; to create the initial \
             delegation, run `joy auth token add {m}` instead.",
            m = member
        );
    }

    let passphrase = read_passphrase(passphrase_flag, "Passphrase: ")?;
    let unlocked = joy_core::auth::unlock_identity(human, &passphrase)?;
    let identity_seed = unlocked.seed;

    // Rotation rewrites the per-(operator, AI) delegation salt; the
    // privkey is then re-derivable from the operator's passphrase plus
    // the new salt at every issuance. No on-disk persistence.
    let project_id = session::project_id(&root)?;
    let new_salt = generate_salt();
    let new_seed =
        delegation::derive_delegation_seed(&identity_seed, &new_salt, &project_id, member);
    let new_kp = IdentityKeypair::from_seed(&new_seed);

    // Update project.yaml: new delegation_verifier + new delegation_salt +
    // rotated timestamp. Single write; the old keypair is unreachable after
    // this point. Legacy entries (no delegation_salt under ADR-033 §1) gain
    // the salt here, which unblocks future multi-machine bootstrap on
    // every machine downstream.
    let project_path = store::joy_dir(&root).join(store::PROJECT_FILE);
    let mut project_mut = store::read_project(&project_path)?;
    let entry = project_mut
        .members
        .get_mut(&email)
        .and_then(|m| m.ai_delegations.get_mut(member))
        .expect("delegation entry exists -- validated above");
    entry.delegation_verifier = new_kp.public_key().to_hex();
    entry.delegation_salt = Some(new_salt.to_hex());
    entry.rotated = Some(Utc::now());
    store::write_yaml_preserve(&project_path, &project_mut)?;
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    joy_core::git_ops::auto_git_add(&root, &[&rel]);

    // Clear any local session file for the AI member. If the AI runs on a
    // different machine this is a no-op; if it shares the machine, the
    // stale session file would fail signature verification on next use
    // anyway - removing it makes the local state visibly consistent.
    let _ = session::remove_session(&project_id, member);

    println!("Rotated delegation for {member}.");
    println!();
    println!("Any prior tokens and any sessions bound to them are invalidated.");
    println!("Issue a fresh token with `joy auth token add {member}`.");

    joy_core::git_ops::auto_git_post_command(&root, &format!("ai rotate {}", member), &email);

    Ok(())
}
