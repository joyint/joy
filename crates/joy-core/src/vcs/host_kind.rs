// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Who is at the other end of this operation (design D1.1).
//!
//! Every rule about asking a question hangs off this one word, and the
//! word is a PARAMETER: the host that starts the work names it once, at
//! its own entry point, and the engine never guesses it again. It is
//! not a TTY test - `joy_process::headless()` answers false on every
//! unix host, so a server worker would look interactive there.
//!
//! - joy-cli names it in `cli_main`: `Delegated` when `JOY_SESSION`
//!   names a live delegation session, `Interactive` for a command a
//!   person typed, `Background` for a hook.
//! - the desktop names `Interactive` for a foreground action and
//!   `Background` for the sync worker and the chat poll.
//! - the platform names `Background` everywhere.

/// Who is at the other end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HostKind {
    /// A person started this and can answer a question.
    Interactive,
    /// Nobody is there: a worker, a poll, a hook. THE DEFAULT, because
    /// a host that has not said who it is must not be handed a prompt
    /// that nobody will ever answer.
    #[default]
    Background,
    /// An agent acts for a person inside a delegation session. Never
    /// asked anything either: the person is not watching this process.
    Delegated,
}

impl HostKind {
    /// Whether joy may raise a question a person has to answer
    /// (design D1.10). The whole prompt rule is this one line.
    pub fn may_prompt(self) -> bool {
        matches!(self, HostKind::Interactive)
    }

    /// The word on the wire: the value of `--host-kind` for a plugin
    /// call and the word in an error detail.
    pub fn as_str(self) -> &'static str {
        match self {
            HostKind::Interactive => "interactive",
            HostKind::Background => "background",
            HostKind::Delegated => "delegated",
        }
    }

    /// The word back, `None` for anything else. Nobody defaults a
    /// misspelled kind to `Interactive` by accident.
    pub fn from_word(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "interactive" => Some(HostKind::Interactive),
            "background" => Some(HostKind::Background),
            "delegated" => Some(HostKind::Delegated),
            _ => None,
        }
    }
}

impl std::fmt::Display for HostKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_interactive_host_may_be_asked() {
        assert!(HostKind::Interactive.may_prompt());
        assert!(!HostKind::Background.may_prompt());
        assert!(!HostKind::Delegated.may_prompt());
    }

    #[test]
    fn the_unnamed_host_is_the_quiet_one() {
        assert_eq!(HostKind::default(), HostKind::Background);
        assert!(!HostKind::default().may_prompt());
    }

    #[test]
    fn the_wire_word_round_trips_and_nothing_else_parses() {
        for kind in [
            HostKind::Interactive,
            HostKind::Background,
            HostKind::Delegated,
        ] {
            assert_eq!(HostKind::from_word(kind.as_str()), Some(kind));
        }
        assert_eq!(
            HostKind::from_word("  Delegated "),
            Some(HostKind::Delegated)
        );
        assert_eq!(HostKind::from_word("interactiv"), None);
        assert_eq!(HostKind::from_word(""), None);
    }
}
