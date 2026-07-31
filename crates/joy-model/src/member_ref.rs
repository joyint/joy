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
//! wrapped in [`MemberRef`]; its [`Display`](std::fmt::Display) and (in
//! presentation mode) its `Serialize` yield the *resolved* value, never the raw
//! id. Code that needs the raw id for an internal purpose (map lookup, verifier
//! math) reaches for the explicit [`MemberRef::id`]. A forgotten output path
//! therefore fails safe: it shows the resolved value (or an auth request),
//! never the id.
//!
//! WHO resolves is not this crate's business. The rule above is; the data
//! behind it (the project's privacy mode and its decrypted `members.yaml`)
//! lives in `joy-core`, which installs a resolver here per command via
//! [`install`]. That keeps this type usable where `joy-core` cannot go, the
//! browser included.

use std::borrow::Borrow;
use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Marker rendered when an opaque id cannot be resolved because `members.yaml`
/// is not unlocked. Never the raw id (fail-safe direction).
pub const AUTH_REQUIRED: &str = "<authenticate to view>";

/// Outcome of resolving a member id for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// A concrete display value (name or e-mail, or a non-PII id like `ai:..`).
    Value(String),
    /// Anonymous mode, members.yaml not unlocked: request authentication.
    AuthRequired,
}

/// What a resolver does: turn one raw at-rest id into what a person may see.
pub type Resolver = Rc<dyn Fn(&str) -> Resolved>;

thread_local! {
    /// The resolver for the current command. `None` means none is installed
    /// (e.g. a unit test); then a `MemberRef` displays its raw value, which
    /// is correct for the open-mode case where the value IS the e-mail. The
    /// CLI always installs one.
    static RESOLVER: RefCell<Option<Resolver>> = const { RefCell::new(None) };
    /// Whether serialization should emit the resolved presentation value
    /// (`--json` output) rather than the raw at-rest id (on-disk persistence).
    /// Defaults to persistence so file writes are never corrupted.
    static PRESENTATION: Cell<bool> = const { Cell::new(false) };
}

/// Install the resolver for the current command (call once at dispatch).
pub fn install(resolver: Resolver) {
    RESOLVER.with(|r| *r.borrow_mut() = Some(resolver));
}

/// Remove any installed resolver (mainly for tests).
pub fn uninstall() {
    RESOLVER.with(|r| *r.borrow_mut() = None);
}

/// Whether serialization is currently in presentation mode (`--json` output).
/// Lets a `serialize_with` on a map keyed by raw ids resolve its keys for
/// output while keeping them raw on disk.
pub fn presentation_active() -> bool {
    PRESENTATION.with(|p| p.get())
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
        Some(resolve) => match resolve(raw) {
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

    /// Alias of [`id`](Self::id): the raw at-rest string. Inherent so it shadows
    /// the unstable `str::as_str` reachable through `Deref`.
    pub fn as_str(&self) -> &str {
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

// Lets `BTreeMap<MemberRef, _>::get(&string)` and `contains_key(&string)` keep
// working with a `&String` key without rewriting every call site.
impl Borrow<String> for MemberRef {
    fn borrow(&self) -> &String {
        &self.0
    }
}

// Deref to the raw id is what makes the conversion tractable: existing
// `&str`-taking call sites (map lookups, comparisons, internal helpers) keep
// compiling via deref coercion. This does NOT weaken the output guarantee:
// every output channel resolves independently of Deref -- terminal via
// `Display`/`color::user`, `--json` via the presentation-aware `Serialize`.
// Deref yields the raw id only where code explicitly coerces to `&str`, which
// by convention is internal use (the same role as the explicit `id()`).
impl std::ops::Deref for MemberRef {
    type Target = str;
    fn deref(&self) -> &str {
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

    /// A resolver like the anonymous one joy-core installs: a table of
    /// display values, and "authenticate" for anything not in it.
    fn table(entries: &[(&str, &str)]) -> Resolver {
        let map: std::collections::BTreeMap<String, String> = entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Rc::new(move |id: &str| match map.get(id) {
            Some(v) => Resolved::Value(v.clone()),
            None => Resolved::AuthRequired,
        })
    }

    #[test]
    fn without_a_resolver_the_value_passes_through() {
        uninstall();
        assert_eq!(
            MemberRef::new("horst@joydev.com").to_string(),
            "horst@joydev.com"
        );
    }

    #[test]
    fn an_unresolvable_id_asks_for_authentication_and_never_shows_itself() {
        install(table(&[]));
        let shown = MemberRef::new("m-secret").to_string();
        assert_eq!(shown, AUTH_REQUIRED);
        assert!(!shown.contains("m-secret"));
        uninstall();
    }

    #[test]
    fn the_compound_delegated_by_form_resolves_both_sides() {
        install(table(&[
            ("ai:claude@joy", "ai:claude@joy"),
            ("m-human", "human@joydev.com"),
        ]));
        let m = MemberRef::new("ai:claude@joy delegated-by:m-human");
        assert_eq!(m.to_string(), "ai:claude@joy delegated-by:human@joydev.com");
        uninstall();
    }

    #[test]
    fn serializing_persists_the_raw_id_and_presents_the_resolved_one() {
        install(table(&[("m-abc", "horst@joydev.com")]));
        let m = MemberRef::new("m-abc");
        assert_eq!(serde_json::to_string(&m).unwrap(), "\"m-abc\"");
        with_presentation(|| {
            assert_eq!(serde_json::to_string(&m).unwrap(), "\"horst@joydev.com\"");
        });
        uninstall();
    }
}
