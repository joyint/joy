// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! `MemberRef`: the only way a member identity reaches Joy's output (ADR-042).
//!
//! In anonymous mode the at-rest member key is an opaque id (`m-<short>`). The
//! concept (Pseudonyme Identitäten und Löschrecht) requires that this id is
//! *never* user-visible: every Joy output that names a member resolves it to a
//! name (if `members.yaml` carries one) or the e-mail, and a path that cannot
//! resolve requests authentication instead of printing the id.
//!
//! The guarantee is bound to the type, not to each output path. A member id is
//! wrapped in [`MemberRef`]; its [`Display`] and (in presentation mode) its
//! `Serialize` yield the *resolved* value, never the raw id. Code that needs
//! the raw id for an internal purpose (map lookup, verifier math) reaches for
//! the explicit [`MemberRef::id`]. A forgotten output path therefore fails
//! safe: it shows the resolved value (or an auth request), never the id.
//!
//! Resolution reads from a thread-local [`MemberResolver`] installed once per
//! command (see [`install`]). In open mode the resolver is a pass-through (the
//! key *is* the e-mail). In anonymous mode it carries the decrypted
//! `members.yaml`, or `None` when the viewer is not authenticated.

use std::borrow::Borrow;
use std::cell::{Cell, RefCell};
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::members_file::MembersFile;

/// Marker rendered when an opaque id cannot be resolved because `members.yaml`
/// is not unlocked. Never the raw id (fail-safe direction).
pub const AUTH_REQUIRED: &str = "<authenticate to view>";

thread_local! {
    /// The resolver for the current command. `None` means no resolver is
    /// installed (e.g. a joy-core unit test); then a `MemberRef` displays its
    /// raw value, which is correct for the open-mode case where the value is
    /// the e-mail. The CLI always installs one (see `install`).
    static RESOLVER: RefCell<Option<MemberResolver>> = const { RefCell::new(None) };
    /// Whether serialization should emit the resolved presentation value
    /// (`--json` output) rather than the raw at-rest id (on-disk persistence).
    /// Defaults to persistence so file writes are never corrupted.
    static PRESENTATION: Cell<bool> = const { Cell::new(false) };
}

/// Outcome of resolving a member id for display.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Resolved {
    /// A concrete display value (name or e-mail, or a non-PII id like `ai:..`).
    Value(String),
    /// Anonymous mode, members.yaml not unlocked: request authentication.
    AuthRequired,
}

/// Resolves opaque member ids to a display value for the current command.
///
/// Built once (from the project privacy mode and, in anonymous mode, the
/// decrypted `members.yaml`) and installed via [`install`].
#[derive(Debug, Clone, Default)]
pub struct MemberResolver {
    anonymous: bool,
    members: Option<MembersFile>,
}

impl MemberResolver {
    /// Open-mode resolver: every key already is the e-mail, so resolution is a
    /// pass-through.
    pub fn open() -> Self {
        Self {
            anonymous: false,
            members: None,
        }
    }

    /// Anonymous-mode resolver. `members` is the decrypted `members.yaml` when
    /// the viewer is authenticated, or `None` when it is locked.
    pub fn anonymous(members: Option<MembersFile>) -> Self {
        Self {
            anonymous: true,
            members,
        }
    }

    fn resolve(&self, id: &str) -> Resolved {
        // Open mode: the id is the e-mail already.
        if !self.anonymous {
            return Resolved::Value(id.to_string());
        }
        // AI members keep a readable synthetic id and carry no PII; show as-is.
        if crate::model::project::is_ai_member(id) {
            return Resolved::Value(id.to_string());
        }
        match &self.members {
            // Unlocked: name, else e-mail. An id absent from members.yaml (e.g.
            // an erased member, GDPR Art. 17) has no e-mail left to show, so the
            // opaque id is all that remains and is not PII.
            Some(m) => Resolved::Value(m.display_for(id).unwrap_or_else(|| id.to_string())),
            // Locked: never the id.
            None => Resolved::AuthRequired,
        }
    }
}

/// Install the resolver for the current command (call once at dispatch).
pub fn install(resolver: MemberResolver) {
    RESOLVER.with(|r| *r.borrow_mut() = Some(resolver));
}

/// Remove any installed resolver (mainly for tests).
pub fn uninstall() {
    RESOLVER.with(|r| *r.borrow_mut() = None);
}

/// Run `f` with serialization in presentation mode, so `MemberRef` serializes
/// the resolved value instead of the raw id. Used by the `--json` emitter.
pub fn with_presentation<T>(f: impl FnOnce() -> T) -> T {
    PRESENTATION.with(|p| p.set(true));
    let out = f();
    PRESENTATION.with(|p| p.set(false));
    out
}

/// Resolve a raw member string for display against the installed resolver.
///
/// This is the generic chokepoint: any output helper that renders a member
/// string (e.g. `color::user`) routes it through here, so resolution is applied
/// in one place rather than per command. Handles the event-log compound form
/// `"<actor> delegated-by:<human>"`. With no resolver installed it passes
/// through (correct for open mode and unit tests).
pub fn resolve_str(raw: &str) -> String {
    resolve_for_display(raw)
}

/// Resolve a bare id against the installed resolver. Handles the event-log
/// compound form `"<actor> delegated-by:<human>"` by resolving each side.
fn resolve_for_display(raw: &str) -> String {
    if let Some((actor, human)) = raw.split_once(" delegated-by:") {
        return format!(
            "{} delegated-by:{}",
            resolve_for_display(actor),
            resolve_for_display(human)
        );
    }
    RESOLVER.with(|r| match &*r.borrow() {
        Some(resolver) => match resolver.resolve(raw) {
            Resolved::Value(v) => v,
            Resolved::AuthRequired => AUTH_REQUIRED.to_string(),
        },
        // No resolver installed: pass through. Correct for open-mode contexts
        // (the value is the e-mail) and for unit tests.
        None => raw.to_string(),
    })
}

/// A member identity that resolves to a display value on output and never
/// exposes the raw at-rest id except via [`MemberRef::id`].
#[derive(Debug, Clone, Default)]
pub struct MemberRef(String);

impl MemberRef {
    /// Wrap a raw at-rest member key (e-mail in open mode, opaque id in
    /// anonymous mode).
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// The raw at-rest id. Use only on internal paths (map lookup, verifier
    /// math), never to build output.
    pub fn id(&self) -> &str {
        &self.0
    }

    /// Consume into the raw id string.
    pub fn into_id(self) -> String {
        self.0
    }

    /// The resolved display value for the current resolver (name, e-mail, or an
    /// auth request). This is what `Display` renders.
    pub fn display(&self) -> String {
        resolve_for_display(&self.0)
    }
}

impl fmt::Display for MemberRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&resolve_for_display(&self.0))
    }
}

impl From<String> for MemberRef {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for MemberRef {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl Borrow<str> for MemberRef {
    fn borrow(&self) -> &str {
        &self.0
    }
}

// Identity/equality/order are over the raw id (the at-rest key), independent of
// resolution, so MemberRef can serve as a map key and compare against ids.
impl PartialEq for MemberRef {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for MemberRef {}
impl PartialEq<str> for MemberRef {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}
impl PartialEq<&str> for MemberRef {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}
impl PartialOrd for MemberRef {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for MemberRef {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}
impl std::hash::Hash for MemberRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl Serialize for MemberRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Presentation mode (--json output) emits the resolved value so an
        // opaque id never leaves Joy; persistence mode (the default, used by all
        // on-disk writes) emits the raw id so files round-trip unchanged.
        if PRESENTATION.with(|p| p.get()) {
            serializer.serialize_str(&resolve_for_display(&self.0))
        } else {
            serializer.serialize_str(&self.0)
        }
    }
}

impl<'de> Deserialize<'de> for MemberRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(String::deserialize(deserializer)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::members_file::{MemberInfo, MembersFile};

    fn members_with(id: &str, email: &str, name: Option<&str>) -> MembersFile {
        let mut mf = MembersFile::default();
        mf.members.insert(
            id.to_string(),
            MemberInfo {
                email: email.to_string(),
                name: name.map(str::to_string),
            },
        );
        mf
    }

    #[test]
    fn open_mode_passes_through() {
        install(MemberResolver::open());
        let m = MemberRef::new("horst@joydev.com");
        assert_eq!(m.to_string(), "horst@joydev.com");
        uninstall();
    }

    #[test]
    fn anonymous_unlocked_resolves_to_email_then_name() {
        install(MemberResolver::anonymous(Some(members_with(
            "m-abc",
            "horst@joydev.com",
            None,
        ))));
        assert_eq!(MemberRef::new("m-abc").to_string(), "horst@joydev.com");
        uninstall();

        install(MemberResolver::anonymous(Some(members_with(
            "m-abc",
            "horst@joydev.com",
            Some("Horst Jens"),
        ))));
        assert_eq!(MemberRef::new("m-abc").to_string(), "Horst Jens");
        uninstall();
    }

    #[test]
    fn anonymous_locked_requests_auth_never_id() {
        install(MemberResolver::anonymous(None));
        let shown = MemberRef::new("m-secret").to_string();
        assert_eq!(shown, AUTH_REQUIRED);
        assert!(!shown.contains("m-secret"));
        uninstall();
    }

    #[test]
    fn ai_member_shown_as_is_even_anonymous() {
        install(MemberResolver::anonymous(None));
        assert_eq!(MemberRef::new("ai:claude@joy").to_string(), "ai:claude@joy");
        uninstall();
    }

    #[test]
    fn compound_delegated_by_resolves_both_sides() {
        install(MemberResolver::anonymous(Some({
            let mut mf = members_with("m-ai-op", "op@joydev.com", None);
            mf.members.insert(
                "m-human".into(),
                MemberInfo {
                    email: "human@joydev.com".into(),
                    name: None,
                },
            );
            mf
        })));
        // ai actor stays readable, the delegating human resolves to e-mail.
        let m = MemberRef::new("ai:claude@joy delegated-by:m-human");
        assert_eq!(m.to_string(), "ai:claude@joy delegated-by:human@joydev.com");
        uninstall();
    }

    #[test]
    fn serialize_persists_raw_id_by_default() {
        install(MemberResolver::anonymous(Some(members_with(
            "m-abc",
            "horst@joydev.com",
            None,
        ))));
        let m = MemberRef::new("m-abc");
        // Default (persistence): raw id.
        assert_eq!(serde_json::to_string(&m).unwrap(), "\"m-abc\"");
        // Presentation: resolved.
        with_presentation(|| {
            assert_eq!(serde_json::to_string(&m).unwrap(), "\"horst@joydev.com\"");
        });
        uninstall();
    }

    #[test]
    fn id_returns_raw_for_internal_use() {
        install(MemberResolver::anonymous(Some(members_with(
            "m-abc",
            "horst@joydev.com",
            None,
        ))));
        assert_eq!(MemberRef::new("m-abc").id(), "m-abc");
        uninstall();
    }
}
