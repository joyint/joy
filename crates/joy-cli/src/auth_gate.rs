// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The one place a command proves who is acting (operator decision
//! 2026-09-27, JOY-02B2-65).
//!
//! Before it, three mechanisms sat side by side: the guard refused a
//! write with "run `joy auth`" and offered no way to type the passphrase
//! there; the crypt, member and AI commands asked for their own
//! `--passphrase` and left no session behind, and asked even when a
//! session stood; only the chat commands read the seed a session
//! carries. Every command now goes through [`unlock`] or [`enforce`],
//! and they all answer the same way:
//!
//! 1. A passphrase GIVEN to the call (the global `--passphrase`,
//!    `--passphrase-stdin` or `JOY_PASSPHRASE`) is checked, session or
//!    not: a wrong one is refused, never quietly ignored.
//! 2. Otherwise a live session of this terminal for the acting member
//!    carries the identity seed (`chat_seed`, JOY-0269-BC): nothing is
//!    asked.
//! 3. Otherwise the passphrase is asked at the terminal. A correct
//!    passphrase (given or typed) makes the session `joy auth` would
//!    make, so the next command asks nothing; except under `--user`,
//!    which names a member for one call and remembers nothing.
//! 4. Nothing to ask with (no terminal, no flag): the typed refusal
//!    that names both ways.
//!
//! An AI never comes through here with a passphrase: its identity is
//! its delegation session (`JOY_SESSION`), and a command that needs the
//! HUMAN behind it (an attestation, a zone) asks for the operator, whom
//! [`joy_core::identity::acting_human_key`] names.

use std::path::Path;

use anyhow::Result;
use joy_core::auth::{session, IdentityKeypair};
use joy_core::context::Context;
use joy_core::guard::Action;
use joy_core::model::project::Project;

/// The global `--passphrase`, carried as `JOY_PASSPHRASE`. Empty counts
/// as absent.
pub fn passphrase_flag() -> Option<String> {
    std::env::var("JOY_PASSPHRASE")
        .ok()
        .filter(|s| !s.is_empty())
}

/// The global `--passphrase-stdin`, carried as `JOY_PASSPHRASE_STDIN`.
pub fn passphrase_from_stdin() -> bool {
    std::env::var("JOY_PASSPHRASE_STDIN").is_ok_and(|v| v == "1")
}

/// Whether the call brought a passphrase along, by flag or by stdin, so
/// a command knows it runs non-interactively.
pub fn passphrase_given() -> bool {
    passphrase_flag().is_some() || passphrase_from_stdin()
}

/// The acting member with their identity open: the key that signs and
/// the seed that unwraps zones and chats.
pub struct Unlocked {
    /// The at-rest member key (address in open mode, opaque id in
    /// anonymous mode).
    pub member_key: String,
    pub keypair: IdentityKeypair,
    pub seed: [u8; 32],
}

/// Open the identity of the human this command acts for (see the module
/// doc for the order). `project` is the caller's already loaded project,
/// so the member looked up here is the one the caller will write.
pub fn unlock_acting(root: &Path, project: &Project) -> Result<Unlocked> {
    let member_key = joy_core::identity::acting_human_key(root)?;
    unlock(root, project, &member_key)
}

/// Open the identity of `member_key`, who must be the human this command
/// acts for: their session of this terminal first, then the passphrase.
pub fn unlock(root: &Path, project: &Project, member_key: &str) -> Result<Unlocked> {
    let member = project
        .member_by_key(member_key)
        .ok_or_else(|| anyhow::anyhow!("{} is not a registered project member", member_key))?;
    if member.verify_key.is_none() {
        anyhow::bail!(
            "{} has no identity yet. Run `joy auth init` first.",
            member_key
        );
    }

    // 2. The session of this terminal, if it is this member's and still
    //    carries the seed the login cached, and no passphrase was given
    //    (a given one is checked below, whatever the session says).
    //    `resolve_identity` did the validating (signature, terminal,
    //    expiry); a `--user` on this call answers unauthenticated there,
    //    so a named member always proves themselves below.
    let session = session_seed(root, project, member_key).and_then(|seed| {
        let keypair = IdentityKeypair::from_seed(&seed);
        let expected = member.verify_key.as_deref().unwrap_or_default();
        (keypair.public_key().to_hex() == expected).then_some((keypair, seed))
    });
    if !passphrase_given() {
        if let Some((keypair, seed)) = session {
            joy_core::member_migration::migrate_quietly(root, member_key, &keypair);
            return Ok(Unlocked {
                member_key: member_key.to_string(),
                keypair,
                seed,
            });
        }
    }

    // 1 and 3. The passphrase: flag, stdin, environment, or the terminal.
    let passphrase = crate::commands::auth::read_passphrase("Passphrase: ")?;
    if joy_core::identity::named_user().is_some() || session.is_some() {
        // One call as this member, nothing remembered (operator,
        // 2026-09-27), or a session that already stands and a given
        // passphrase that only had to be right: unwrap and go.
        let unlocked = joy_core::auth::unlock_identity(member, &passphrase)?;
        joy_core::member_migration::migrate_quietly(root, member_key, &unlocked.keypair);
        return Ok(Unlocked {
            member_key: member_key.to_string(),
            keypair: unlocked.keypair,
            seed: unlocked.seed,
        });
    }
    // The same login `joy auth` runs: attestation posture, the session
    // with the seed and the members zone key cached, re-locking of files
    // left open. The next command asks nothing. Said on stderr, and not
    // at all in `--json` mode, whose one envelope is all a caller reads.
    let outcome = joy_core::auth::login::login(root, member_key, &passphrase)?;
    if !crate::output::is_json() {
        eprintln!(
            "Authenticated as {}. Session active (24h).",
            outcome.address
        );
    }
    Ok(Unlocked {
        member_key: outcome.member_key,
        keypair: outcome.keypair,
        seed: outcome.seed,
    })
}

/// Bring a project from before the member files over when the person
/// at this terminal is signed in: their session carries the key that
/// signs what the AI members may do (JI-019D-46). Runs before the
/// command, asks nothing, and does nothing where nobody is signed in or
/// the project already keeps its members in files.
pub fn bring_members_over(root: &Path) {
    if !joy_core::member_migration::pending(root) {
        return;
    }
    let Ok(project) = joy_core::store::load_project(root) else {
        return;
    };
    let Ok(member_key) = joy_core::identity::acting_human_key(root) else {
        return;
    };
    if let Some(seed) = session_seed(root, &project, &member_key) {
        let keypair = IdentityKeypair::from_seed(&seed);
        joy_core::member_migration::migrate_quietly(root, &member_key, &keypair);
    }
}

/// The seed cached in this terminal's session, if that session is the
/// acting member's and valid.
fn session_seed(root: &Path, project: &Project, member_key: &str) -> Option<[u8; 32]> {
    let identity = joy_core::identity::resolve_identity(root).ok()?;
    if !identity.authenticated || identity.member.id() != member_key {
        return None;
    }
    let project_id = session::project_id_of(project);
    let token = session::load_session(&project_id, member_key).ok()??;
    if token.claims.expires <= chrono::Utc::now() {
        return None;
    }
    let bytes = hex::decode(token.chat_seed.as_deref()?).ok()?;
    bytes.try_into().ok()
}

/// Check and enforce a guard action the way [`Context::enforce`] does,
/// but ask for the passphrase first where the guard would otherwise
/// refuse for want of proof. A correct passphrase makes the session (or,
/// under `--user`, proves this one call), and the identity in `ctx` is
/// authenticated from here on.
pub fn enforce(ctx: &mut Context, action: &Action, target: &str) -> Result<()> {
    if ctx.guard().needs_authentication(action, &ctx.identity) {
        let member = ctx.identity.member.id().to_string();
        if member.trim().is_empty() {
            // Nobody to ask for: the typed refusal names both ways.
            return Err(joy_core::error::JoyError::UnknownActingMember.into());
        }
        let project = joy_core::store::load_project(&ctx.root)?;
        unlock(&ctx.root, &project, &member)?;
        ctx.identity.authenticated = true;
    }
    ctx.enforce(action, target)?;
    Ok(())
}

/// [`enforce`] for the commands that hold no [`Context`] of their own.
pub fn enforce_at(root: &Path, action: &Action, target: &str) -> Result<()> {
    let mut ctx = Context::load_at(root)?;
    enforce(&mut ctx, action, target)
}
