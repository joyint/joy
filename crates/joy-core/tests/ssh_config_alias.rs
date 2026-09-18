// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! An ssh config alias is honoured, all the way to the address joy
//! dials (forge connection NG, design D1.4 and D1.5).
//!
//! libgit2 reads no ssh config at all and opens the socket itself from
//! the URL, so `git@work:owner/repo.git` with `Host work / HostName
//! git.example.com / Port 2222` would be looked up as the literal name
//! `work` on port 22 and fail with a DNS error, which names the wrong
//! cause. joy rewrites the address for the contact, keeps the
//! configured remote untouched, and still reads the alias's own
//! `IdentityFile` and `User` while doing it.
//!
//! Its own test binary: it moves HOME, which is process state.

use joy_core::vcs::remote_url::{RemoteUrl, Transport};
use joy_core::vcs::{ssh_auth, ssh_config, HostKind};

fn write(path: &std::path::Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

#[test]
fn an_alias_is_dialled_at_its_real_address_and_keeps_its_own_key() {
    let home = tempfile::tempdir().unwrap();
    let ssh = home.path().join(".ssh");
    write(
        &ssh.join("work_ed25519"),
        "-----BEGIN RSA PRIVATE KEY-----\nMIIB\n-----END RSA PRIVATE KEY-----\n",
    );
    write(
        &ssh.join("config"),
        "Host work\n  HostName git.example.com\n  Port 2222\n  User deploy\n\
         \x20 IdentityFile ~/.ssh/work_ed25519\n\
         \nInclude conf.d/*.conf\n\
         \nHost plain\n  HostName git.example.com\n",
    );
    // A fragment of the kind Ubuntu, Fedora and macOS all ship an
    // Include line for, and that 1Password and corporate setups write.
    write(
        &ssh.join("conf.d").join("10-bastion.conf"),
        "Host inside.corp.invalid\n  ProxyJump gateway.corp.invalid\n",
    );
    std::env::set_var("HOME", home.path());
    std::env::set_var("USERPROFILE", home.path());

    // The scp-like remote keeps its form, because the two forms do not
    // mean the same path: the bracketed form is the only scp-like
    // shape that carries a port (D1.5).
    let dialled = ssh_config::effective_url("git@work:owner/repo.git")
        .expect("the alias names another address");
    assert_eq!(dialled, "[git@git.example.com:2222]:owner/repo.git");
    let parsed = RemoteUrl::parse(&dialled).expect("joy reads back what it wrote");
    assert_eq!(parsed.transport, Transport::Ssh);
    assert_eq!(parsed.host, "git.example.com");
    assert_eq!(parsed.port, Some(2222));
    assert_eq!(parsed.user.as_deref(), Some("git"));
    assert_eq!(
        parsed.path, "owner/repo.git",
        "the scp form's path has no leading slash, and the server is sent it as it stands"
    );

    // The ssh:// form keeps its own shape.
    assert_eq!(
        ssh_config::effective_url("ssh://git@work/owner/repo.git").as_deref(),
        Some("ssh://git@git.example.com:2222/owner/repo.git")
    );
    // A port in the URL is ssh's command line and beats the config.
    assert_eq!(
        ssh_config::effective_url("ssh://git@work:29418/owner/repo.git").as_deref(),
        Some("ssh://git@git.example.com:29418/owner/repo.git")
    );
    // A host the config does not rename is left exactly as written, so
    // the named remote stays the normal case.
    assert_eq!(ssh_config::effective_url("git@github.com:o/r.git"), None);
    assert_eq!(
        ssh_config::effective_url("https://work/owner/repo.git"),
        None,
        "https is not ssh's business"
    );
    // A rename with no port change keeps the plain scp form.
    assert_eq!(
        ssh_config::effective_url("git@plain:o/r.git").as_deref(),
        Some("git@git.example.com:o/r.git")
    );

    // The contact runs against git.example.com, and the settings for
    // it still come from the `Host work` block: the key the person
    // named there, and the user.
    let through_alias = ssh_config::for_contact("git.example.com", Some("git@work:owner/repo.git"));
    assert_eq!(
        through_alias.identity_files,
        vec![ssh.join("work_ed25519")],
        "the alias's own IdentityFile, with ~ expanded"
    );
    assert_eq!(through_alias.user.as_deref(), Some("deploy"));
    // A remote that names the host itself keeps its own block.
    let direct = ssh_config::for_contact("git.example.com", Some("git@git.example.com:o/r.git"));
    assert!(direct.identity_files.is_empty());
    assert_eq!(direct.user, None);

    // And the chain built from the alias's settings offers that key,
    // under the URL's user (ssh's `-l` beats a `User` line).
    let chain = ssh_auth::chain_for(
        "git.example.com",
        Some("git"),
        &through_alias,
        HostKind::Background,
        &ssh_auth::Agent::Missing,
        false,
    );
    assert_eq!(chain.user, "git");
    assert_eq!(
        chain.candidates,
        vec![ssh_auth::SshCandidate::Key {
            path: ssh.join("work_ed25519"),
            public: None,
            passphrase: None,
        }]
    );

    // The Include is read, so a ProxyJump inside a config.d fragment
    // is refused by name instead of being invisible (D1.4).
    assert!(
        ssh_config::refusal_for_url("git@inside.corp.invalid:o/r.git")
            .expect("the fragment's rule counts")
            .contains("ProxyJump")
    );
    // And it belongs to the host that carries it, to no other.
    assert!(ssh_config::refusal_for_url("git@work:o/r.git").is_none());
    assert!(ssh_config::refusal_for_url("git@github.com:o/r.git").is_none());
}
