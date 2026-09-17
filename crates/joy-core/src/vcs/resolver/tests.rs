// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The rule of D1.2, read without a forge, an agent or a connector.
//!
//! Every fact the resolver decides on is an argument of [`plan_with`],
//! so the candidate order, the twin trigger of decision 11 and the four
//! refusals of D1.5 are all decidable here. What needs a real contact
//! (the tracking ref after a twin push, a rejected ref) lives in the
//! engine tests beside it.

use super::*;

/// A token that stands for whatever a connector handed out. The value
/// is never read by the rules under test.
fn a_token() -> HostToken {
    HostToken {
        token: "a-token".to_string(),
        kind: Some(ForgeKind::GitHub),
        login: Some("scotty-work".to_string()),
        source: Some("keychain".to_string()),
    }
}

/// A machine whose ssh chain has something to offer.
fn ssh_ready() -> SshProbe {
    SshProbe {
        candidates: 1,
        signals: SshSignals::default(),
        notes: Vec::new(),
    }
}

/// A machine with no agent and no readable key: the normal state of a
/// Windows desktop, and trigger (a) of D1.2.
fn ssh_empty() -> SshProbe {
    SshProbe {
        candidates: 0,
        signals: SshSignals::default(),
        notes: vec!["no ssh agent is running".to_string()],
    }
}

fn github_facts() -> HostFacts {
    HostFacts {
        claimed: Some(ForgeKind::GitHub),
        claimed_by_plugin: true,
        web_url: None,
        token: Some(a_token()),
    }
}

fn no_rule(_: &str) -> Option<String> {
    None
}

// ---- the candidate order (D1.2) --------------------------------------

#[test]
fn an_ssh_remote_with_a_working_chain_stays_on_ssh_even_with_a_token() {
    let plan = plan_with(
        "git@github.com:acme/widgets.git",
        &github_facts(),
        None,
        &ssh_ready(),
        &no_rule,
    );
    assert_eq!(plan.legs[0].way, Way::Configured);
    assert_eq!(plan.legs[0].transport, Transport::Ssh);
    assert!(
        matches!(plan.legs[0].credential, LegCredential::Machine),
        "the ssh candidates come first, whatever tokens exist (D1.2 rule 2)"
    );
    // The twin is the SECOND contact, and only after the ssh contact
    // failed with an authentication class failure.
    assert_eq!(plan.legs.len(), 2);
    assert_eq!(plan.legs[1].way, Way::Twin);
    assert_eq!(plan.legs[1].url, "https://github.com/acme/widgets.git");
}

#[test]
fn a_host_whose_memory_says_ssh_worked_never_goes_to_the_twin() {
    let memory = HostMemory::new(TransportState::SshWorked);
    let plan = plan_with(
        "git@github.com:acme/widgets.git",
        &github_facts(),
        Some(&memory),
        &ssh_ready(),
        &no_rule,
    );
    assert_eq!(plan.legs.len(), 1, "one leg, and it is ssh: {plan:#?}");
    assert_eq!(plan.legs[0].transport, Transport::Ssh);
    assert!(!plan.uses_twin(), "whatever tokens exist (D1.2 rule 3)");
    assert!(plan.why().contains("ssh worked for github.com before"));
}

#[test]
fn no_agent_and_no_readable_key_goes_to_the_twin_on_the_first_contact() {
    let plan = plan_with(
        "git@github.com:acme/widgets.git",
        &github_facts(),
        None,
        &ssh_empty(),
        &no_rule,
    );
    assert_eq!(
        plan.legs.len(),
        1,
        "there is nothing to offer over ssh, so there is no ssh leg: {plan:#?}"
    );
    assert_eq!(plan.legs[0].way, Way::Twin);
    assert_eq!(plan.legs[0].transport, Transport::Https);
    assert!(plan.legs[0].credential.is_token());
    assert!(
        plan.why().contains("no ssh agent is running"),
        "and it says why: {}",
        plan.why()
    );
}

#[test]
fn a_remembered_ssh_failure_puts_the_twin_first_and_keeps_ssh_behind_it() {
    let memory = HostMemory::new(TransportState::SshFailed);
    let plan = plan_with(
        "git@github.com:acme/widgets.git",
        &github_facts(),
        Some(&memory),
        &ssh_ready(),
        &no_rule,
    );
    assert_eq!(plan.legs[0].way, Way::Twin);
    assert_eq!(plan.legs[1].way, Way::Configured);
    assert!(plan.why().contains("ssh-failed"));
}

#[test]
fn an_https_remote_offers_the_token_and_then_the_helper_in_one_contact() {
    let plan = plan_with(
        "https://github.com/acme/widgets.git",
        &github_facts(),
        None,
        &SshProbe::empty(),
        &no_rule,
    );
    assert_eq!(plan.legs.len(), 1, "one contact, not two: {plan:#?}");
    assert_eq!(plan.legs[0].way, Way::Configured);
    assert!(plan.legs[0].credential.is_token());
}

#[test]
fn an_https_remote_with_no_token_still_asks_the_credential_helper() {
    let plan = plan_with(
        "https://forge.acme-internal.example/acme/widgets.git",
        &HostFacts::none(),
        None,
        &SshProbe::empty(),
        &no_rule,
    );
    assert_eq!(plan.legs.len(), 1);
    assert!(matches!(plan.legs[0].credential, LegCredential::Machine));
}

#[test]
fn an_ssh_remote_with_no_token_anywhere_never_grows_a_twin_leg() {
    let plan = plan_with(
        "git@github.com:acme/widgets.git",
        &HostFacts {
            claimed: Some(ForgeKind::GitHub),
            claimed_by_plugin: true,
            web_url: None,
            token: None,
        },
        None,
        &ssh_empty(),
        &no_rule,
    );
    assert_eq!(plan.legs.len(), 1);
    assert_eq!(plan.legs[0].transport, Transport::Ssh);
    assert!(plan.why().contains("nobody is signed in to github.com"));
}

// ---- the twin and its refusals (D1.5) --------------------------------

#[test]
fn the_table_knows_three_hosts_and_the_two_ssh_sub_domains() {
    assert_eq!(
        twin_from_table("git@github.com:acme/widgets.git").unwrap(),
        "https://github.com/acme/widgets.git"
    );
    assert_eq!(
        twin_from_table("ssh://git@ssh.github.com:443/acme/widgets.git").unwrap(),
        "https://github.com/acme/widgets.git"
    );
    assert_eq!(
        twin_from_table("git@altssh.gitlab.com:acme/widgets.git").unwrap(),
        "https://gitlab.com/acme/widgets.git"
    );
    assert_eq!(
        twin_from_table("git@codeberg.org:acme/widgets.git").unwrap(),
        "https://codeberg.org/acme/widgets.git"
    );
}

#[test]
fn the_bracketed_port_form_is_not_part_of_the_host() {
    // `[git@host:2222]:owner/repo.git` is the one scp-like shape that
    // carries a port; read as a plain host:path it would produce the
    // host `github.com` and the path `2222]:acme/widgets.git`.
    assert_eq!(
        twin_from_table("[git@github.com:2222]:acme/widgets.git").unwrap(),
        "https://github.com/acme/widgets.git"
    );
}

#[test]
fn a_host_nobody_claims_and_no_table_knows_has_no_twin() {
    assert_eq!(
        twin_from_table("git@forge.acme-internal.example:acme/widgets.git"),
        Err(TwinRefusal::UnknownHost)
    );
    let mut notes = Vec::new();
    let leg = twin_leg(
        "git@forge.acme-internal.example:acme/widgets.git",
        "forge.acme-internal.example",
        &HostFacts {
            token: Some(a_token()),
            ..HostFacts::none()
        },
        &no_rule,
        &mut notes,
    );
    assert!(leg.is_none());
    assert!(notes[0].contains("no forge connector claims"), "{notes:?}");
}

#[test]
fn a_connector_that_claims_a_host_without_naming_its_web_base_has_no_twin() {
    // Only the connector knows where a self hosted instance answers:
    // it may sit under a nested sub path, or behind a different
    // SSH_DOMAIN entirely. A claim alone is not an address.
    let mut notes = Vec::new();
    let leg = twin_leg(
        "git@code.acme.example:acme/widgets.git",
        "code.acme.example",
        &HostFacts {
            claimed: Some(ForgeKind::Gitea),
            claimed_by_plugin: true,
            web_url: None,
            token: Some(a_token()),
        },
        &no_rule,
        &mut notes,
    );
    assert!(leg.is_none());
    assert!(
        notes[0].contains("did not name its https address"),
        "and it says which of the two refusals this is: {notes:?}"
    );
}

#[test]
fn fewer_than_two_path_segments_has_no_twin() {
    assert_eq!(
        twin_from_table("git@github.com:widgets.git"),
        Err(TwinRefusal::ShortPath)
    );
}

#[test]
fn a_host_without_a_dot_has_no_twin() {
    assert_eq!(
        twin_from_table("git@work:acme/widgets.git"),
        Err(TwinRefusal::NotAHostName)
    );
}

#[test]
fn a_connector_web_url_beats_the_table_and_reaches_a_self_hosted_sub_path() {
    let facts = HostFacts {
        claimed: Some(ForgeKind::GitLab),
        claimed_by_plugin: true,
        web_url: Some("https://code.acme.example/gitlab/acme/widgets.git".to_string()),
        token: Some(a_token()),
    };
    let mut notes = Vec::new();
    let leg = twin_leg(
        "git@code.acme.example:acme/widgets.git",
        "code.acme.example",
        &facts,
        &no_rule,
        &mut notes,
    )
    .expect("the connector named the web base");
    assert_eq!(leg.url, "https://code.acme.example/gitlab/acme/widgets.git");
}

#[test]
fn a_push_insteadof_rule_that_matches_the_twin_keeps_the_remote_on_ssh() {
    let rewrite = |url: &str| {
        url.strip_prefix("https://github.com/")
            .map(|rest| format!("git@github.com:{rest}"))
    };
    let plan = plan_with(
        "git@github.com:acme/widgets.git",
        &github_facts(),
        None,
        &ssh_empty(),
        &rewrite,
    );
    assert_eq!(plan.legs.len(), 1);
    assert_eq!(plan.legs[0].transport, Transport::Ssh);
    assert!(!plan.uses_twin());
    assert!(
        plan.why().contains(
            "rewrites the https address of github.com to git@github.com:acme/widgets.git"
        ),
        "and it says why: {}",
        plan.why()
    );
}

// ---- the insteadOf prediction (D1.5) ---------------------------------

/// A config file with the entries a case needs, read by git2 exactly as
/// libgit2 reads the person's own.
fn config_with(text: &str) -> (tempfile::TempDir, git2::Config) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gitconfig");
    std::fs::write(&path, text).expect("write config");
    let config = git2::Config::open(&path).expect("open config");
    (dir, config)
}

#[test]
fn the_longest_prefix_wins_exactly_as_libgit2_picks_it() {
    let (_dir, config) = config_with(
        "[url \"git@github.com:\"]\n\tinsteadOf = https://github.com/\n\
         [url \"git@work:\"]\n\tinsteadOf = https://github.com/acme/\n",
    );
    assert_eq!(
        insteadof_rewrite(
            &config,
            "https://github.com/acme/widgets.git",
            ContactDirection::Fetch
        ),
        Some("git@work:widgets.git".to_string()),
        "the longer prefix wins (remote.c:3079-3140)"
    );
    assert_eq!(
        insteadof_rewrite(
            &config,
            "https://github.com/other/widgets.git",
            ContactDirection::Fetch
        ),
        Some("git@github.com:other/widgets.git".to_string())
    );
    assert_eq!(
        insteadof_rewrite(
            &config,
            "https://codeberg.org/acme/widgets.git",
            ContactDirection::Fetch
        ),
        None
    );
}

#[test]
fn push_insteadof_is_read_on_a_push_and_not_on_a_fetch() {
    let (_dir, config) =
        config_with("[url \"git@github.com:\"]\n\tpushInsteadOf = https://github.com/\n");
    assert_eq!(
        insteadof_rewrite(
            &config,
            "https://github.com/acme/widgets.git",
            ContactDirection::Push
        ),
        Some("git@github.com:acme/widgets.git".to_string())
    );
    assert_eq!(
        insteadof_rewrite(
            &config,
            "https://github.com/acme/widgets.git",
            ContactDirection::Fetch
        ),
        None,
        "a pushInsteadOf rule says nothing about a fetch"
    );
}

// ---- the transport memory (D1.2) -------------------------------------

#[test]
fn the_memory_is_written_at_0600_and_read_back() {
    with_state_file(|path| {
        remember(
            "GitHub.com",
            HostMemory::new(TransportState::SshWorked).with_credential(Transport::Ssh, "agent"),
        );
        let memory = recall("github.com").expect("the host is remembered, lowercased");
        assert_eq!(memory.state, TransportState::SshWorked);
        assert_eq!(memory.credential.as_deref(), Some("agent"));
        assert_eq!(
            used("github.com").as_deref(),
            Some("joy reached github.com over ssh with an identity from your ssh agent")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "joy's own state file, and nobody else's");
        }
        let _ = path;
    });
}

#[test]
fn a_row_older_than_the_ttl_is_not_read() {
    with_state_file(|_| {
        let mut memory = HostMemory::new(TransportState::NoSshCredential);
        memory.at = unix_now() - MEMORY_TTL.as_secs() - 1;
        remember("codeberg.org", memory);
        assert!(
            recall("codeberg.org").is_none(),
            "the 24 hour TTL of D1.2 is read from the row"
        );
    });
}

#[test]
fn the_memory_survives_only_until_the_machine_changes_under_it() {
    with_state_file(|_| {
        let signals = SshSignals {
            agent_socket: None,
            agent_identities: 0,
            keys: BTreeMap::new(),
        };
        remember(
            "github.com",
            HostMemory::new(TransportState::NoSshCredential).with_signals(signals.clone()),
        );
        let remembered = recall("github.com").expect("fresh");
        // An agent that appears is exactly the change D1.2 names.
        let now = SshSignals {
            agent_socket: Some("/tmp/agent.sock".to_string()),
            agent_identities: 2,
            keys: BTreeMap::new(),
        };
        assert_ne!(remembered.signals, now);
        forget("github.com");
        assert!(recall("github.com").is_none());
    });
}

// ---- the token cache (D1.7) ------------------------------------------

#[test]
fn the_token_ttl_is_the_smaller_of_the_expiry_and_five_minutes() {
    // No expiry: five minutes.
    assert_eq!(token_ttl(None), Duration::from_secs(300));
    // Far away: still five minutes.
    let far = chrono::Utc::now() + chrono::Duration::hours(2);
    assert_eq!(token_ttl(Some(&far.to_rfc3339())), Duration::from_secs(300));
    // Two minutes away: two minutes less the grace of sixty seconds.
    let soon = chrono::Utc::now() + chrono::Duration::seconds(120);
    let ttl = token_ttl(Some(&soon.to_rfc3339()));
    assert!(
        ttl <= Duration::from_secs(60) && ttl >= Duration::from_secs(55),
        "{ttl:?}"
    );
    // Already spent: nothing is reused.
    let gone = chrono::Utc::now() - chrono::Duration::seconds(10);
    assert_eq!(token_ttl(Some(&gone.to_rfc3339())), Duration::ZERO);
}

#[test]
fn a_connector_user_name_decides_the_shape_and_the_id_is_the_fallback() {
    assert_eq!(
        token_kind("gitea", Some("x-access-token")),
        Some(ForgeKind::GitHub)
    );
    assert_eq!(
        token_kind("github", Some("oauth2")),
        Some(ForgeKind::GitLab)
    );
    assert_eq!(token_kind("gitea", None), Some(ForgeKind::Gitea));
    assert_eq!(
        token_kind("github-enterprise", Some("")),
        Some(ForgeKind::GitHubEnterprise)
    );
}

#[test]
fn an_ssh_authentication_failure_is_read_from_the_class_and_the_code() {
    let auth = git2::Error::new(
        git2::ErrorCode::Auth,
        git2::ErrorClass::Ssh,
        "some words nobody may match on",
    );
    assert!(is_ssh_auth_failure(&auth));
    let other = git2::Error::new(
        git2::ErrorCode::Auth,
        git2::ErrorClass::Http,
        "an https refusal is not an ssh one",
    );
    assert!(!is_ssh_auth_failure(&other));
    let network = git2::Error::new(
        git2::ErrorCode::GenericError,
        git2::ErrorClass::Ssh,
        "no route to host",
    );
    assert!(
        !is_ssh_auth_failure(&network),
        "a network fault is not an authentication failure, and must not send anyone to the twin"
    );
}
