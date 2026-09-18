// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The host kind under a REAL delegation session (D1.1 of the forge
//! connection NG design, JOY-0297-1A).
//!
//! The case that matters is the agent on a terminal: it has stdin and
//! stdout on a PTY, so the terminal answer alone would call it
//! `Interactive` and joy would ask it to type an address. A live session
//! must beat the terminal. Proving that needs a session that really
//! loads, not a `JOY_SESSION` value that merely exists, so this test
//! mints one on disk.
//!
//! ONE test in its own binary: `JOY_SESSION` and the state directory that
//! holds the session file are process state.

use chrono::Duration;
use joy_core::auth::session;
use joy_core::auth::IdentityKeypair;
use joy_core::host::HostKind;

/// Mint a session for `member` in the isolated state directory and return
/// the `JOY_SESSION` value that carries it, exactly as the token
/// redemption hands it to an agent (ADR-033).
fn live_session(member: &str, ttl: Duration) -> String {
    let ephemeral_private = [7u8; 32];
    let ephemeral = IdentityKeypair::from_seed(&ephemeral_private);
    let token = session::create_session_for_ai(
        &ephemeral,
        member,
        "DLG",
        Some(ttl),
        &ephemeral.public_key().to_hex(),
        None,
        Some("human@example.com".to_string()),
    );
    session::save_session("DLG", &token).unwrap();
    let sid = session::session_storage_id("DLG", &token.claims);
    session::encode_session_env(&sid, &ephemeral_private)
}

#[test]
fn a_live_session_makes_the_host_delegated_even_on_a_terminal() {
    let state = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", state.path());
    std::env::set_var("XDG_STATE_HOME", state.path().join(".state"));

    // Nothing in the environment: the terminal decides, and only it.
    std::env::remove_var("JOY_SESSION");
    assert_eq!(HostKind::detect(true), HostKind::Interactive);
    assert_eq!(HostKind::detect(false), HostKind::Background);

    // A live delegation session: the agent owns a terminal and is still
    // an agent. This is the case J9's refusal rule is written for, and it
    // is the one the value alone cannot prove.
    std::env::set_var(
        "JOY_SESSION",
        live_session("ai:claude@joy", Duration::hours(1)),
    );
    assert_eq!(HostKind::detect(true), HostKind::Delegated);
    assert_eq!(HostKind::detect(false), HostKind::Delegated);
    assert!(!HostKind::detect(true).may_ask());

    // An expired session is no delegation: the process is what it was
    // before, so a person at the terminal is served rather than refused.
    std::env::set_var(
        "JOY_SESSION",
        live_session("ai:claude@joy", Duration::minutes(-5)),
    );
    assert_eq!(HostKind::detect(true), HostKind::Interactive);
    assert_eq!(HostKind::detect(false), HostKind::Background);

    // A leftover value that names no session at all is no delegation
    // either. This is what a test harness sets when it writes
    // `JOY_SESSION=sid:0000`, and why such a value proves nothing about
    // the delegated branch.
    std::env::set_var("JOY_SESSION", "sid:0000");
    assert_eq!(HostKind::detect(true), HostKind::Interactive);
    std::env::set_var("JOY_SESSION", "");
    assert_eq!(HostKind::detect(true), HostKind::Interactive);
}
