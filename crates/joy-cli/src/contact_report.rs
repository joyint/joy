// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! ONE failure vocabulary for every CLI command that contacts a forge
//! (design D3.8).
//!
//! Before this, each command classified its own failure from the text a
//! git process had printed: `joy chat` had `classify_sync_error` with
//! four substring rules, `joy release` had none at all. Two commands
//! therefore gave a person two different names for the same refusal, and
//! an agent had nothing stable to read.
//!
//! Now the engine classifies (`vcs::contact`, D1.8a) and this module
//! says the verdict. The shape is the CLI's own and does not change with
//! the command:
//!
//! ```text
//! <what did not happen>: <the one plain sentence>
//!   = note: state <word>; <what happens next>
//!   = help: <the one next step>
//!   = detail: <libgit2's own words, the operation, the sources tried>
//! ```
//!
//! `--json` and stdout. A contact that IS the command's answer prints
//! the envelope the rest of the CLI prints and exits 1
//! ([`Refusal::fail`]). A contact that is NOT the command's answer (the
//! chat delivery after a send, which is best effort and never fatal)
//! may not write to stdout at all in `--json` mode, because stdout
//! carries exactly one envelope (ADR-036) and a second object there is a
//! corrupt answer. It goes to stderr as one JSON object instead, with
//! the same fields ([`Refusal::say_aside`]), so an agent reads the same
//! words wherever the contact sat.
//!
//! The rule binds the COMMAND and not only this module. A command whose
//! answer is the envelope says its own progress on stderr in that mode,
//! because one line in front of the object makes the object unparsable:
//! `Pushing to origin...{"version":1,...}` is not an answer an agent can
//! read (D3.10; `joy release publish` does it through its own `say`).

use serde::Serialize;

use joy_core::error::JoyError;
use joy_core::vcs::contact;

use crate::output;

/// One failed forge contact, in the words a person and an agent both
/// read.
#[derive(Debug, Clone, Serialize)]
pub struct Refusal {
    /// The forge host joy really contacted.
    pub host: String,
    /// The state word of D1.8a: `needs_sign_in`, `offline`, ...
    pub state: &'static str,
    /// The one plain sentence for that state. Never libgit2's text.
    pub message: String,
    /// The one next step, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// libgit2's own words and the operation, for a person who has to
    /// debug a state none of the rules recognised.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Refusal {
    /// The refusal behind a [`JoyError`] from a forge contact.
    ///
    /// `host` is the forge joy contacted, which the caller reads off the
    /// checkout's own remote: the classifier's sentence names it, and a
    /// sentence that names the wrong host sends the person to sign in
    /// somewhere joy never talks to (D1.1).
    pub fn of(host: &str, error: &JoyError) -> Self {
        let failure = error.failure();
        let contact = error.contact();
        Self {
            host: host.to_string(),
            state: failure.reason(),
            // The classifier's own sentence when it wrote one, else the
            // state's. Both are plain; neither is libgit2's.
            message: contact
                .map(|c| c.message.clone())
                .unwrap_or_else(|| failure.sentence(host)),
            action: contact
                .and_then(|c| c.action.clone())
                .or_else(|| crate::commands::forge::action_line(failure, host)),
            detail: contact.and_then(|c| c.detail.clone()),
        }
    }

    /// Say a refusal that is NOT the command's answer: the chat
    /// delivery after a send, the healing push of a read. The command
    /// itself succeeded, so nothing here changes the exit code and
    /// nothing here writes to stdout.
    ///
    /// `headline` says what did not happen, `tail` what happens next.
    pub fn say_aside(&self, headline: &str, tail: &str) {
        if output::is_json() {
            // One line, on stderr, so the command's own stdout stays
            // exactly one envelope.
            if let Ok(line) = serde_json::to_string(self) {
                eprintln!("{line}");
                return;
            }
        }
        eprintln!("{headline}: {}", self.message);
        eprintln!("  = note: state {}; {tail}", self.state);
        if let Some(action) = &self.action {
            eprintln!("  = help: {action}");
        }
        if let Some(detail) = &self.detail {
            eprintln!("  = detail: {detail}");
        }
    }
}

impl Refusal {
    /// End the command with this refusal, which is what a command whose
    /// ANSWER is the contact does (`joy release publish`). In `--json`
    /// mode the envelope goes to stdout and the process exits 1,
    /// exactly as `joy forge login` does; otherwise the lines become
    /// the error the CLI prints.
    pub fn fail(self) -> anyhow::Error {
        if output::is_json() {
            let _ = output::emit(&self);
            std::process::exit(1);
        }
        let mut text = format!("{}\n  = note: state {}", self.message, self.state);
        if let Some(action) = &self.action {
            text.push_str(&format!("\n  = help: {action}"));
        }
        if let Some(detail) = &self.detail {
            text.push_str(&format!("\n  = detail: {detail}"));
        }
        anyhow::anyhow!(text)
    }
}

/// The forge host of this checkout: the remote joy really contacts,
/// `origin` or else the first one (D1.1). Empty when there is none, and
/// the sentences then name no host rather than the wrong one.
pub fn host_of_checkout(root: &std::path::Path) -> String {
    joy_core::vcs::forge::remote_url(root)
        .map(|url| contact::host_of(&url))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use joy_core::vcs::contact::{ContactError, Failure};

    fn contact_error(failure: Failure, message: &str, detail: Option<&str>) -> JoyError {
        JoyError::Contact(Box::new(ContactError {
            failure,
            message: message.to_string(),
            detail: detail.map(str::to_string),
            action: None,
            next_try: None,
            self_imposed: false,
        }))
    }

    /// The state word and the sentence come from the classifier, not
    /// from the text libgit2 printed: that text only ever reaches the
    /// detail line.
    #[test]
    fn the_refusal_speaks_the_classifier_s_words() {
        let error = contact_error(
            Failure::NeedsSignIn,
            "Sign in to GitHub to sync.",
            Some("chats push; too many redirects or authentication replays"),
        );
        let refusal = Refusal::of("github.com", &error);
        assert_eq!(refusal.state, "needs_sign_in");
        assert_eq!(refusal.message, "Sign in to GitHub to sync.");
        assert_eq!(
            refusal.action.as_deref(),
            Some("run `joy forge login --host github.com`")
        );
        assert_eq!(
            refusal.detail.as_deref(),
            Some("chats push; too many redirects or authentication replays")
        );
    }

    /// An error that never went through a contact still gets a state,
    /// and it is `error` rather than a guess at prose.
    #[test]
    fn a_local_fault_is_error_and_not_offline() {
        let error = JoyError::Git("the index has unresolved conflicts".into());
        let refusal = Refusal::of("codeberg.org", &error);
        assert_eq!(refusal.state, "error");
        assert_eq!(refusal.message, Failure::Error.sentence("codeberg.org"));
        assert!(refusal.detail.is_none());
    }

    /// JOY-02A9-48: what the person reads when the organisation has not
    /// approved Joy. One plain sentence, and the page an owner acts on
    /// as the one next step: the state's own next step ("open the
    /// approval page") is a button label and no use on a terminal,
    /// while the URL is what a person can follow from here.
    #[test]
    fn the_organisation_wall_names_the_approval_page() {
        let page = "https://github.com/organizations/acme/settings/oauth_application_policy";
        let error = JoyError::Contact(Box::new(ContactError {
            failure: Failure::NeedsOrgApproval,
            message: Failure::NeedsOrgApproval.sentence("github.com"),
            detail: None,
            action: Some(page.to_string()),
            next_try: None,
            self_imposed: false,
        }));
        let refusal = Refusal::of("github.com", &error);
        assert_eq!(refusal.state, "needs_org_approval");
        assert_eq!(
            refusal.message,
            "Your organisation must approve Joy for this repository."
        );
        assert_eq!(refusal.action.as_deref(), Some(page));
    }

    /// A state that names no next step gets no help line rather than a
    /// wrong one.
    #[test]
    fn a_state_without_a_step_offers_none() {
        let error = contact_error(Failure::RateLimited, "Codeberg is rate limiting us.", None);
        let refusal = Refusal::of("codeberg.org", &error);
        assert_eq!(refusal.state, "rate_limited");
        assert!(refusal.action.is_none());
    }

    /// The JSON shape an agent reads: the five fields, and the two that
    /// have no answer are absent rather than null.
    #[test]
    fn the_json_shape_carries_the_state() {
        let error = contact_error(Failure::Offline, "No connection to codeberg.org.", None);
        let refusal = Refusal::of("codeberg.org", &error);
        let json = serde_json::to_string(&refusal).unwrap();
        assert!(json.contains(r#""state":"offline""#), "{json}");
        assert!(json.contains(r#""host":"codeberg.org""#), "{json}");
        assert!(!json.contains("detail"), "{json}");
    }
}
