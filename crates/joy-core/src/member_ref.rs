// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Who resolves an opaque member id, and to what (ADR-042).
//!
//! The TYPE and the guarantee live in `joy-model`, so they also hold where
//! joy-core cannot go (the browser). What lives here is the data behind the
//! resolution: the project's privacy mode and, in anonymous mode, the
//! decrypted `members.yaml`. [`install`] hands that to joy-model as a
//! closure, once per command.

use std::rc::Rc;

use crate::members_file::MembersFile;

pub use joy_model::member_ref::{
    presentation_active, resolve_str, uninstall, with_presentation, MemberRef, Resolved,
    AUTH_REQUIRED,
};

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
    joy_model::member_ref::install(Rc::new(move |id: &str| resolver.resolve(id)));
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
