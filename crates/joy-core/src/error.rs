// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum JoyError {
    #[error("project already initialized at {0}")]
    AlreadyInitialized(PathBuf),

    /// No founding address and nobody to ask for one: the named refusal of
    /// the forge connection NG design (D3.9). A host that CAN ask a person
    /// asks instead of raising this.
    #[error("this project does not know who you are; run joy init --user <address>")]
    NoFounderIdentity,

    /// A write needs an acting member and none is known: no delegation
    /// session, no git config naming a member, and no forge account
    /// naming one either (operator decision 2026-09-19, JOY-02AE-1A,
    /// correcting D3.9 of package J11). A checkout whose `user.email`
    /// names a member is answered by that address now; this error is for
    /// the one that answers nothing at all.
    ///
    /// The first line is the sentence D4.5 writes, and it is the whole
    /// remedy in the app, whose host has a member picker. The rest is
    /// marked as the command line's own, because this error reaches the
    /// command line too, and from commands that take no `--user` of their
    /// own (`joy crypt`, `joy deauth`, `joy auth passphrase`). Setting
    /// `git config user.email` to a registered member's address is the
    /// remedy that needs no further command; authenticating once
    /// (`joy auth --user <address>`) works on a checkout with no git
    /// config at all.
    #[error(
        "this project does not know who you are, pick your member\n\
         In the app that is the member picker. On the command line, set \
         git config user.email to a registered member's address \
         (git config user.email <address>), or name yourself once: joy \
         auth --user <address>, or joy auth init --user <address> when \
         this member has no passphrase here yet."
    )]
    UnknownActingMember,

    #[error(
        "{0} is a forge alias address, not an identity (JOY-0253-8A).\n\
         Set your real address first:\n  git config user.email \"you@example.com\"\n\
         or pass --user you@example.com"
    )]
    FounderAliasIdentity(String),

    #[error("no Joy project found (run `joy init` first)")]
    NotInitialized,

    #[error("item not found: {0}")]
    ItemNotFound(String),

    #[error("milestone not found: {0}")]
    MilestoneNotFound(String),

    #[error("circular dependency detected: {0}")]
    CircularDependency(String),

    #[error("failed to create directory {path}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to write {path}")]
    WriteFile {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to read {path}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("{path}: {source}")]
    YamlParse {
        path: PathBuf,
        source: serde_yaml_ng::Error,
    },

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml_ng::Error),

    #[error("git error: {0}")]
    Git(String),

    /// A forge contact that failed, carried TYPED (JOY-02A3-E4): the
    /// classifier's verdict travels with the error instead of being
    /// flattened into prose, so a surface names the state and offers
    /// its one action (D1.8b) rather than parsing a sentence.
    ///
    /// `Display` is the plain sentence the person reads and nothing
    /// else; libgit2's own words and the sources joy tried stay on the
    /// error's detail line, which a details view opens deliberately.
    #[error("{0}")]
    Contact(Box<crate::vcs::contact::ContactError>),

    #[error("template error: {0}")]
    Template(String),

    #[error("authentication failed: {0}")]
    AuthFailed(String),

    /// An anonymous project could not name the member who just proved
    /// their identity (ADR-042). The member map keys them by an opaque
    /// id, and the address behind it lives only in the encrypted
    /// `members.yaml`: without that file there is nothing to check the
    /// attestation against, because an attestation never signs the id,
    /// and nothing to tell the person they authenticated AS.
    ///
    /// Said out loud rather than answered with the id. Falling back to
    /// the id produces an attestation check that fails with "the entry
    /// appears to have been tampered with", which points a person at
    /// their own entry when the truth is a missing file.
    #[error(
        "this project cannot name the member {0}: its encrypted members.yaml \
         could not be opened.\n\
         Anonymous mode keeps every address there, and that address is what \
         an attestation signs and what a login tells you. Check that \
         .joy/members.yaml is present and current (git pull), or ask a \
         manage member to re-add you."
    )]
    AnonymousMemberUnnamed(String),

    #[error("crypto error: {0}")]
    Crypto(joy_crypt::Error),

    #[error("no access to zone '{zone}': ask a member with access to run `joy crypt grant`")]
    ZoneAccessDenied { zone: String },

    #[error("passphrase too short (minimum 3 words)")]
    PassphraseTooShort,

    #[error("guard denied: {0}")]
    GuardDenied(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}

impl JoyError {
    /// The verdict behind this error when it is a failed forge contact:
    /// the state for the banner, the sentence, the detail line and the
    /// moment the forge serves again (D1.8b). `None` for every other
    /// error, and a caller that only wants the state can read
    /// [`crate::vcs::contact::Failure::Error`] for those.
    pub fn contact(&self) -> Option<&crate::vcs::contact::ContactError> {
        match self {
            JoyError::Contact(contact) => Some(contact),
            _ => None,
        }
    }

    /// The state a surface shows for this error: the classifier's
    /// verdict when the error came from a contact, and `error`
    /// otherwise - never guessed at from prose (D1.8a).
    pub fn failure(&self) -> crate::vcs::contact::Failure {
        self.contact()
            .map(|contact| contact.failure)
            .unwrap_or(crate::vcs::contact::Failure::Error)
    }
}

impl From<joy_crypt::Error> for JoyError {
    /// Map crypt-level errors onto joy-core's domain errors. A missing
    /// zone key surfaces the domain `ZoneAccessDenied` (with its
    /// actionable "ask a member to run `joy crypt grant`" message),
    /// exactly as before the primitives moved to joy-crypt (JI-014D-D8);
    /// every other crypto failure wraps as `Crypto`.
    fn from(e: joy_crypt::Error) -> Self {
        match e {
            joy_crypt::Error::ZoneKeyUnavailable { zone } => JoyError::ZoneAccessDenied { zone },
            other => JoyError::Crypto(other),
        }
    }
}

impl From<joy_token::TokenError> for JoyError {
    /// Delegation-token validation/decoding failures carry a user-facing
    /// hint; surface them as `AuthFailed` (JI-0175-B0), the same variant
    /// the token code used before it moved to the wasm-portable crate.
    fn from(e: joy_token::TokenError) -> Self {
        JoyError::AuthFailed(e.0)
    }
}
