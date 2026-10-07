// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Who resolves an opaque member id, and to what (ADR-042).
//!
//! The TYPE and the guarantee live in `joy-model`, so they also hold where
//! joy-core cannot go (the browser). What lives here is the data behind the
//! resolution: the project's privacy mode and, in anonymous mode, the
//! decrypted `members.yaml`. [`install`] hands that to joy-model as a
//! closure, once per command.

use std::path::Path;
use std::rc::Rc;

use joy_crypt::zone::ZoneKey;

use crate::members_file::MembersFile;
use crate::model::project::PrivacyMode;

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

    /// The decrypted members file behind an unlocked anonymous resolver,
    /// for a host that needs name and address apart (the desktop's member
    /// list, JOY-02C3-85). None in open mode and while locked.
    pub fn members(&self) -> Option<&MembersFile> {
        self.members.as_ref()
    }

    /// Whether this resolver would answer [`Resolved::AuthRequired`] for a
    /// human id: anonymous mode with no members file at hand.
    pub fn locked(&self) -> bool {
        self.anonymous && self.members.is_none()
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

/// The resolver for the project at `root`, built the one way every host
/// builds it (JOY-02C3-85): an open project passes ids through; an
/// anonymous project decrypts `members.yaml` with the members-zone key
/// the acting member's live session caches, and is locked without one.
/// A directory that is no project resolves like an open one, since there
/// are no opaque ids in it to protect.
///
/// The CLI installed this per command and the desktop never did, so the
/// same anonymous project named its members in a terminal and showed
/// opaque ids in the app (operator validation 2026-10-07).
pub fn resolver_for(root: &Path) -> MemberResolver {
    let Ok(project) = crate::store::load_project(root) else {
        return MemberResolver::open();
    };
    if project.privacy_mode() != PrivacyMode::Anonymous {
        return MemberResolver::open();
    }
    MemberResolver::anonymous(session_members(root))
}

/// [`resolver_for`], installed for the current command on this thread.
/// Returns the resolver too, for a host that also reads it directly.
pub fn install_for(root: &Path) -> MemberResolver {
    let resolver = resolver_for(root);
    install(resolver.clone());
    resolver
}

/// `members.yaml` decrypted with the members-zone key cached in the
/// acting member's live session; None when there is no live session, the
/// session carries no key, or the file does not read.
fn session_members(root: &Path) -> Option<MembersFile> {
    let identity = crate::identity::resolve_identity(root).ok()?;
    let project_id = crate::auth::session::project_id(root).ok()?;
    let token = crate::auth::session::load_session(&project_id, &identity.member).ok()??;
    if token.claims.expires <= chrono::Utc::now() {
        return None;
    }
    let bytes = hex::decode(token.members_zone_key.as_deref()?).ok()?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    crate::members_file::read(root, &ZoneKey::from_bytes(arr)).ok()
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

    /// A directory that is no project protects no ids (JOY-02C3-85).
    #[test]
    fn a_directory_without_a_project_resolves_openly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let resolver = resolver_for(dir.path());
        assert!(!resolver.locked());
        install(resolver);
        assert_eq!(MemberRef::new("m-deadbeef").to_string(), "m-deadbeef");
        uninstall();
    }

    /// An anonymous project on a machine where nobody signed in is
    /// locked: the id stays behind the sign-in affordance, never shown
    /// (JOY-02C3-85).
    #[test]
    fn an_anonymous_project_without_a_session_is_locked() {
        let dir = tempfile::tempdir().expect("tempdir");
        let joy = crate::store::joy_dir(dir.path());
        std::fs::create_dir_all(&joy).expect("joy dir");
        // the project file as `joy init --anonymous` leaves it: the model
        // keeps the switch behind a migration, so the mode is written here
        let project = crate::model::project::Project::new("Anon".into(), Some("AN".into()));
        let yaml = serde_yaml_ng::to_string(&project).expect("yaml");
        std::fs::write(
            joy.join(crate::store::PROJECT_FILE),
            format!("{yaml}privacy: anonymous\n"),
        )
        .expect("project file");
        assert_eq!(
            crate::store::load_project(dir.path())
                .expect("loads")
                .privacy_mode(),
            PrivacyMode::Anonymous
        );
        let resolver = resolver_for(dir.path());
        assert!(resolver.locked(), "no session, no members file: locked");
        assert!(resolver.members().is_none());
        install(resolver);
        assert_eq!(MemberRef::new("m-deadbeef").to_string(), AUTH_REQUIRED);
        uninstall();
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
