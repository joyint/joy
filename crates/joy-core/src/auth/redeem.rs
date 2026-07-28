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

    // The delegating human and their registered verify key. `delegated_by`
    // carries whichever identifier the issuer had in hand: the raw git
    // e-mail (`joy auth token add`) or the already-resolved at-rest member
    // key (the app's attested add, which never touches a raw e-mail in
    // anonymous mode, ADR-042). Accept BOTH — resolving an at-rest key
    // through the e-mail matcher fails in anonymous mode, which silently
    // broke redemption for every app-issued token there.
    let human = &delegation.claims.delegated_by;
    let human_member = project
        .member_by_key(human)
        .or_else(|| project.member_by_email(human))
        .ok_or_else(|| {
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

    // The delegation key the token carries: it is the AI's identity for
    // everything it does with it, chats and zones alike. It must match the
    // registered verifier or the delegation was rotated since issuance.
    let delegation_private: Option<[u8; 32]> = {
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
            // A token issued before the key rode along: it can still
            // authenticate, and whatever needs the key says so where it
            // is needed.
            None => None,
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::token::{create_token, encode_token, TokenIssueParams, TokenSigningKeys};
    use crate::model::project::{AiDelegationEntry, Member, MemberCapabilities};

    const AI: &str = "ai:claude@joy";
    const HUMAN: &str = "human@example.com";
    const PID: &str = "TST";

    // A project where HUMAN has delegated to AI, wired so a crypt token
    // from that delegation redeems. Returns (project, delegation_seed).
    fn project_with_delegation() -> (Project, [u8; 32], IdentityKeypair) {
        let delegator = IdentityKeypair::from_seed(&[3u8; 32]);
        let delegation_seed = [4u8; 32];
        let delegation = IdentityKeypair::from_seed(&delegation_seed);

        let mut project = Project::new("Test".into(), Some(PID.into()));
        project
            .register_member(AI, Member::new(MemberCapabilities::All))
            .unwrap();
        let mut human = Member::new(MemberCapabilities::All);
        human.verify_key = Some(delegator.public_key().to_hex());
        human.ai_delegations.insert(
            AI.to_string(),
            AiDelegationEntry {
                delegation_verifier: delegation.public_key().to_hex(),
                delegation_salt: Some("00".repeat(32)),
                created: chrono::Utc::now(),
                rotated: None,
            },
        );
        project.register_member(HUMAN, human).unwrap();
        (project, delegation_seed, delegator)
    }

    fn crypt_token(delegator: &IdentityKeypair, seed: &[u8; 32]) -> String {
        let delegation = IdentityKeypair::from_seed(seed);
        let token = create_token(
            TokenSigningKeys {
                delegator,
                delegation: &delegation,
                delegation_seed: seed,
            },
            TokenIssueParams {
                ai_member: AI,
                human: HUMAN,
                project_id: PID,
                ttl: None,
            },
        );
        encode_token(&token)
    }

    // ADR-042 / anonymous mode: an app-issued token names the operator by
    // their AT-REST member key, not a raw e-mail. Resolving that through
    // the e-mail matcher finds nothing, so redemption used to fail for
    // every token the app issued in an anonymous project.
    #[test]
    fn a_token_naming_the_operator_by_their_at_rest_key_redeems() {
        let (project, seed, delegator) = project_with_delegation();
        let delegation = IdentityKeypair::from_seed(&seed);
        // The at-rest key IS the map key; in open mode it equals the
        // e-mail, in anonymous mode it is the opaque id — either way this
        // is what the app has in hand when it signs.
        let token = encode_token(&create_token(
            TokenSigningKeys {
                delegator: &delegator,
                delegation: &delegation,
                delegation_seed: &seed,
            },
            TokenIssueParams {
                ai_member: AI,
                human: HUMAN, // the at-rest key of the human member
                project_id: PID,
                ttl: None,
            },
        ));
        let redeemed = redeem_ai_session(&project, PID, &token).expect("redeems by key");
        assert_eq!(redeemed.member, AI);

        // …and an unknown identifier is still refused, by either route.
        let bogus = encode_token(&create_token(
            TokenSigningKeys {
                delegator: &delegator,
                delegation: &delegation,
                delegation_seed: &seed,
            },
            TokenIssueParams {
                ai_member: AI,
                human: "nobody@example.com",
                project_id: PID,
                ttl: None,
            },
        ));
        assert!(redeem_ai_session(&project, PID, &bogus).is_err());
    }

    #[test]
    fn crypt_token_redeems_to_a_resolvable_ai_session() {
        let (project, seed, delegator) = project_with_delegation();
        let token = crypt_token(&delegator, &seed);

        let redeemed = redeem_ai_session(&project, PID, &token).unwrap();
        assert_eq!(redeemed.member, AI);
        assert_eq!(redeemed.delegated_by, HUMAN);
        assert!(redeemed.session_env.starts_with("joy_s_"));

        // The env carries the delegation private key (crypt scope), so a
        // job/chat container can open sealed chats as the AI member.
        let (_sid, _eph, deleg) = session::parse_session_env_full(&redeemed.session_env).unwrap();
        assert_eq!(deleg, Some(seed), "crypt scope embeds the delegation key");

        // The session carries the F2 delegated_by claim and passes the F3
        // live-delegation gate against this project.
        assert_eq!(redeemed.token.claims.delegated_by.as_deref(), Some(HUMAN));
        assert!(crate::identity::token_session_rejection(
            &project,
            &redeemed.token,
            deleg.as_ref()
        )
        .is_none());
    }

    #[test]
    fn redeem_rejected_when_delegation_absent() {
        // A project that knows the AI but carries no delegation for it.
        let mut project = Project::new("Test".into(), Some(PID.into()));
        project
            .register_member(AI, Member::new(MemberCapabilities::All))
            .unwrap();
        let delegator = IdentityKeypair::from_seed(&[3u8; 32]);
        let token = crypt_token(&delegator, &[4u8; 32]);
        assert!(redeem_ai_session(&project, PID, &token).is_err());
    }
}
