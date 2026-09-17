// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Who is behind this process (D1.1 of the forge connection NG design).
//!
//! Every joy host decides ONCE, at its entry point, whether a person can
//! answer a question: the CLI in `cli_main`, the desktop for a foreground
//! action, the platform never. The engine below takes the answer as a
//! parameter and never reads the environment again, so "may I ask?" has
//! one owner per process instead of a guess per call site.

/// The three hosts joy runs under.
///
/// [`HostKind::Background`] is the default on purpose: a host that says
/// nothing gets the careful behaviour (refuse by name, never wait for an
/// answer nobody is there to give).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    /// A person typed this command and is watching the terminal.
    Interactive,
    /// No person: a hook, a worker, a server.
    #[default]
    Background,
    /// An agent acting under a live delegation session (ADR-033).
    Delegated,
}

impl HostKind {
    /// The kind of THIS process, decided once by its entry point.
    ///
    /// `terminal` is the host's own answer to "can I ask a person here?"
    /// (for the CLI: stdin and stdout are a terminal and the run is not
    /// `--json`). A live `JOY_SESSION` wins over it: an agent that happens
    /// to own a terminal is still an agent. This is the one read of the
    /// environment, and it reads joy's own session variable, not a switch.
    pub fn detect(terminal: bool) -> Self {
        if delegation_session_is_live() {
            HostKind::Delegated
        } else if terminal {
            HostKind::Interactive
        } else {
            HostKind::Background
        }
    }

    /// Whether this host may ask a person a question and wait for the answer.
    pub fn may_ask(self) -> bool {
        matches!(self, HostKind::Interactive)
    }

    /// The word the plugin protocol and the logs use.
    pub fn as_str(self) -> &'static str {
        match self {
            HostKind::Interactive => "interactive",
            HostKind::Background => "background",
            HostKind::Delegated => "delegated",
        }
    }
}

/// The kind the entry point of THIS process decided, set once.
static PROCESS_HOST: std::sync::OnceLock<HostKind> = std::sync::OnceLock::new();

/// Record the host kind of this process. Called exactly once, by the
/// entry point of each host (D1.1): joy-cli in `cli_main`, the desktop
/// when it starts an action, the platform never (it stays
/// [`HostKind::Background`]). A second call is ignored, so no library
/// code can talk a process into believing a person is present.
pub fn set_process_host(kind: HostKind) {
    let _ = PROCESS_HOST.set(kind);
}

/// The host kind of this process. [`HostKind::Background`] until an entry
/// point said otherwise, which is the careful answer for every caller
/// that never decided.
pub fn process_host() -> HostKind {
    PROCESS_HOST.get().copied().unwrap_or_default()
}

/// Whether `JOY_SESSION` names a delegation session that is still alive.
/// A leftover or malformed value is not a delegation: it makes the process
/// no less interactive than it already was.
fn delegation_session_is_live() -> bool {
    let Some(value) = std::env::var("JOY_SESSION").ok().filter(|s| !s.is_empty()) else {
        return false;
    };
    let Some((sid, _, _)) = crate::auth::session::parse_session_env_full(&value) else {
        return false;
    };
    matches!(
        crate::auth::session::load_session_by_id(&sid),
        Ok(Some(session)) if session.claims.expires > chrono::Utc::now()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_terminal_without_a_session_is_interactive() {
        // Without JOY_SESSION the terminal answer decides, and only it.
        // A developer shell that carries a session is not a failure of
        // this rule, so the case skips itself there.
        if std::env::var_os("JOY_SESSION").is_some() {
            return;
        }
        assert_eq!(HostKind::detect(true), HostKind::Interactive);
        assert_eq!(HostKind::detect(false), HostKind::Background);
    }

    #[test]
    fn the_default_host_asks_nobody() {
        assert_eq!(HostKind::default(), HostKind::Background);
        assert!(!HostKind::default().may_ask());
        assert!(HostKind::Interactive.may_ask());
        assert!(!HostKind::Delegated.may_ask());
    }
}
