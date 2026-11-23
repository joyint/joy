// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Platform members (JI-01A7-0F): a platform instance that runs the jobs
//! of AI members is a member of the project, named by its domain, with
//! its public key and the capability `jobs`.
//!
//! A person with the manage capability adds it and signs for it
//! ([`super::vouch`]). The key does two things: a job's token is sealed
//! for it ([`super::job_token`]), and the platform proves with it that
//! it is the member it says ([`identity`]). Nothing is assigned to a
//! platform and nobody delegates to one; what it may write itself is
//! two things, a job of its own back to new and a comment
//! ([`crate::guard`]).

use super::{vouch, IdentityKeypair, PublicKey};
use crate::identity::Identity;
use crate::model::item::Capability;
use crate::model::project::{
    Member, MemberCapabilities, MemberFileShape, MemberKind, MemberLayout, Project,
};

/// Where a platform serves its public key: `https://<domain>` and this
/// path, answering with the hex-encoded Ed25519 key and nothing else.
pub const KEY_PATH: &str = "/.well-known/joy-platform-key";

/// Whether the project keeps its members in a way that can hold a
/// platform member and a job's token: member files that say their type
/// (JOY-02DB-09). The sentence is what a person reads otherwise.
pub fn fits(project: &Project) -> Result<(), String> {
    let in_shape = project.member_layout() == MemberLayout::Files
        && project.member_file_shape() == MemberFileShape::Identity;
    if in_shape {
        Ok(())
    } else {
        Err(crate::migrations::needs_an_update(
            "A platform that runs jobs",
        ))
    }
}

/// The domain a platform is named by, from the address a person gives:
/// `https://app.joyint.com` is `app.joyint.com`. Only https, because
/// the key fetched from there is what the person signs for; a port
/// stays part of the name.
pub fn domain_of(url: &str) -> Result<String, String> {
    let rest = url
        .trim()
        .strip_prefix("https://")
        .ok_or_else(|| "a platform is added by its https address".to_string())?;
    let domain = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // A dot or a port: what tells a domain from the name of an AI member
    // where only an id is at hand (`joy_model::is_ai_member`).
    let plain = (domain.contains('.') || domain.contains(':'))
        && !domain.contains('@')
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
    if plain {
        Ok(domain.to_ascii_lowercase())
    } else {
        Err(format!("{url} is not the address of a platform"))
    }
}

/// The address a platform's key is fetched from.
pub fn key_url(domain: &str) -> String {
    format!("https://{domain}{KEY_PATH}")
}

/// A public key as a platform serves it, checked: the hex of an Ed25519
/// key, whatever whitespace surrounds it.
pub fn read_key(served: &str) -> Result<String, String> {
    let hex = served.trim();
    PublicKey::from_hex(hex)
        .map(|_| hex.to_ascii_lowercase())
        .map_err(|_| "what the platform serves there is not its public key".to_string())
}

/// A platform member as it is added, before anybody has signed for it:
/// its key and the capability jobs.
pub fn new_member(verify_key: &str) -> Member {
    let mut member = Member::of_kind(
        MemberKind::Platform,
        MemberCapabilities::Specific([(Capability::Jobs, Default::default())].into()),
    );
    member.verify_key = Some(verify_key.to_string());
    member
}

/// The platform members of the project, by their domains.
pub fn platforms(project: &Project) -> Vec<String> {
    project
        .members()
        .filter(|(_, member)| member.kind == MemberKind::Platform)
        .map(|(key, _)| key.clone())
        .collect()
}

/// The public key of the platform member `domain`, checked against the
/// signature of the person who added it.
pub fn key_of(project: &Project, domain: &str) -> Result<PublicKey, String> {
    let member = project
        .member_by_key(domain)
        .filter(|member| member.kind == MemberKind::Platform)
        .ok_or_else(|| format!("{domain} is not a platform member of this project"))?;
    vouch::verify_platform(project, domain, member)
}

/// The platform a job runs on when a person opens it: the one they
/// name, else the one the job presets, else the only one there is.
pub fn for_job(
    project: &Project,
    preset: Option<&str>,
    named: Option<&str>,
) -> Result<String, String> {
    let all = platforms(project);
    if all.is_empty() {
        return Err("no platform runs jobs for this project: sign in on a platform first".into());
    }
    let wanted = named.or(preset);
    match wanted {
        Some(domain) => all
            .iter()
            .find(|have| have.as_str() == domain)
            .cloned()
            .ok_or_else(|| format!("{domain} is not a platform member of this project")),
        None if all.len() == 1 => Ok(all[0].clone()),
        None => Err(format!(
            "this project has more than one platform, name the one that runs the job: {}",
            all.join(", ")
        )),
    }
}

/// The platform `domain` as it acts itself, proven by its own key: the
/// identity of its two writes. Refused when the key is not the one a
/// person signed for under that domain.
pub fn identity(
    project: &Project,
    domain: &str,
    keypair: &IdentityKeypair,
) -> Result<Identity, String> {
    let signed_for = key_of(project, domain)?;
    if signed_for.to_hex() != keypair.public_key().to_hex() {
        return Err(format!(
            "this platform's key is not the one {domain} was added to the project with"
        ));
    }
    Ok(Identity {
        member: domain.to_string().into(),
        delegated_by: None,
        authenticated: true,
        grant: None,
        job: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_platform_is_named_by_the_domain_of_its_https_address() {
        assert_eq!(
            domain_of("https://app.joyint.com").unwrap(),
            "app.joyint.com"
        );
        assert_eq!(
            domain_of(" https://App.Joyint.com/login?x=1 ").unwrap(),
            "app.joyint.com"
        );
        assert_eq!(
            domain_of("https://localhost:8443/").unwrap(),
            "localhost:8443"
        );
        assert_eq!(
            domain_of("http://app.joyint.com").unwrap_err(),
            "a platform is added by its https address"
        );
        assert!(domain_of("https://").is_err());
        assert!(domain_of("https://intranet").is_err());
        assert!(!joy_model::is_ai_member("app.joyint.com"));
        assert!(!joy_model::is_ai_member("localhost:8443"));
        assert!(domain_of("https://user@evil.example").is_err());
        assert_eq!(
            key_url("app.joyint.com"),
            "https://app.joyint.com/.well-known/joy-platform-key"
        );
    }

    #[test]
    fn a_served_key_is_a_public_key_or_nothing() {
        let key = IdentityKeypair::from_seed(&[7u8; 32]).public_key().to_hex();
        assert_eq!(read_key(&format!("{key}\n")).unwrap(), key);
        assert!(read_key("<html>not found</html>").is_err());
        assert!(read_key("").is_err());
    }
}
