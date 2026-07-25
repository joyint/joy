// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Redeem a delegation token into an AI session — the shared core of
//! `joy auth --token` and the platform's in-process redemption
//! (JI-0175-B0).
//!
//! The platform running a project's jobs and chat turns is, in this
//! model, an ordinary delegation-token holder: it redeems the operator's
//! token exactly the way the CLI does, with NO platform-specific code
//! path in joy-core. This module is that single path. It performs the
//! validation and builds the session + `JOY_SESSION` env value; the
//! caller decides where to save the session file (the CLI into the local
//! state dir, the platform into a job/chat workspace via
//! `XDG_STATE_HOME`) and whether to log.

use crate::auth::session::{self, SessionToken};
use crate::auth::{token, IdentityKeypair, PublicKey};
use crate::error::JoyError;
use crate::model::project::Project;

/// The result of redeeming a delegation token: everything a caller needs
/// to persist the session and hand its handle to a joy CLI.
pub struct RedeemedSession {
    /// The signed session token to persist with
    /// [`session::save_session`] (in whatever `XDG_STATE_HOME` the caller
    /// has arranged).
    pub token: SessionToken,
    /// The `joy_s_...` value for the consumer's `JOY_SESSION` env var. A
    /// credential — never log it.
    pub session_env: String,
    /// The AI member the session acts as.
    pub member: String,
    /// The delegating operator's e-mail, from the token claims (for
    /// display and audit; resolve to an at-rest key before writing it into
    /// a repo).
    pub delegated_by: String,
}

/// Validate `token_str` against `project` and build an AI session from it.
///
/// Mirrors the CLI's `joy auth --token`: dual-signature + expiry + project
/// validation, AI-member registration check, an ephemeral per-session
/// keypair for proof of possession, and — for `crypt`-scoped tokens — the
/// embedded delegation private key propagated through `JOY_SESSION` so the
/// session can unwrap zone keys and open sealed chats (ADR-041 §5).
///
/// Does NOT touch disk or the event log: the caller persists the returned
/// [`RedeemedSession::token`] where it belongs and logs as it sees fit.
pub fn redeem_ai_session(
    project: &Project,
    project_id: &str,
    token_str: &str,
) -> Result<RedeemedSession, JoyError> {
    let delegation = token::decode_token(token_str)?;

    // The delegating human and their registered verify key.
    let human = &delegation.claims.delegated_by;
    let human_member = project.member_by_email(human).ok_or_else(|| {
        JoyError::AuthFailed(format!("delegating member {human} is not registered"))
    })?;
    let human_pk_hex = human_member.verify_key.as_ref().ok_or_else(|| {
        JoyError::AuthFailed(format!(
            "delegating member {human} has no public key registered"
        ))
    })?;
    let human_pk = PublicKey::from_hex(human_pk_hex)?;

    // The stable delegation entry for this AI member under that operator.
    let ai_member_id = &delegation.claims.ai_member;
    let delegation_entry = human_member
        .ai_delegations
        .get(ai_member_id)
        .ok_or_else(|| {
            JoyError::AuthFailed(format!(
                "no delegation registered for {ai_member_id} by {human}"
            ))
        })?;
    let delegation_pk = PublicKey::from_hex(&delegation_entry.delegation_verifier)?;

    // Dual signatures + project + expiry. Tokens are multi-use within TTL.
    let claims = token::validate_token(&delegation, &human_pk, &delegation_pk, project_id)?;

    if !project.has_member_key(&claims.ai_member) {
        return Err(JoyError::AuthFailed(format!(
            "AI member {} is not registered in this project",
            claims.ai_member
        )));
    }

    // Ephemeral per-session keypair (ADR-033): its private half rides only
    // in JOY_SESSION and proves possession; its public half is recorded in
    // the claims.
    let ephemeral_keypair = IdentityKeypair::from_random();
    let ephemeral_private = ephemeral_keypair.to_seed_bytes();

    // Crypt scope (ADR-041 §5): carry the delegation private key so the
    // session can unwrap zone keys and open sealed chats. It must match the
    // registered verifier or the delegation was rotated since issuance.
    let delegation_private: Option<[u8; 32]> = if claims.has_crypt_scope() {
        match delegation.delegation_private_key.as_ref() {
            Some(hex_seed) => {
                let bytes = hex::decode(hex_seed).map_err(|e| {
                    JoyError::AuthFailed(format!("token has malformed delegation_private_key: {e}"))
                })?;
                let seed: [u8; 32] = bytes.try_into().map_err(|_| {
                    JoyError::AuthFailed("token's delegation_private_key is not 32 bytes".into())
                })?;
                if IdentityKeypair::from_seed(&seed).public_key().to_hex()
                    != delegation_entry.delegation_verifier
                {
                    return Err(JoyError::AuthFailed(
                        "token's delegation_private_key does not match the registered \
                         delegation_verifier; the delegation may have been rotated"
                            .into(),
                    ));
                }
                Some(seed)
            }
            None => {
                return Err(JoyError::AuthFailed(
                    "token claims the crypt scope but carries no delegation private key".into(),
                ))
            }
        }
    } else {
        None
    };

    // Record the delegating human in the signed claims (F2). Bound the
    // session to the token expiry (ADR-041 §6).
    let delegated_by_at_rest = crate::privacy::delegated_by_at_rest(project, human);
    let token_obj = session::create_session_for_ai(
        &ephemeral_keypair,
        &claims.ai_member,
        project_id,
        None,
        &delegation_entry.delegation_verifier,
        claims.expires,
        delegated_by_at_rest,
    );

    let sid = session::session_storage_id(project_id, &token_obj.claims);
    let session_env =
        session::encode_session_env_full(&sid, &ephemeral_private, delegation_private.as_ref());

    Ok(RedeemedSession {
        token: token_obj,
        session_env,
        member: claims.ai_member.clone(),
        delegated_by: claims.delegated_by.clone(),
    })
}
