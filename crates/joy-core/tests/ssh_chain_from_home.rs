// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The ssh chain as a person's home directory presents it (forge
//! connection NG, design D1.4).
//!
//! The two facts this pins down are the two that used to end a whole
//! operation: a key file libssh2 cannot read is `LIBSSH2_ERROR_FILE`,
//! which libgit2 turns into -1 and which kills the fetch instead of
//! moving to the next candidate, and "no agent" is indistinguishable
//! from "agent refused" once libgit2 has folded both into `GIT_EAUTH`.
//!
//! Its own test binary: it moves HOME and `SSH_AUTH_SOCK`, which are
//! process state.

use joy_core::vcs::ssh_auth::{self, Agent, SshCandidate};
use joy_core::vcs::{ssh_config, HostKind};

/// An `openssh-key-v1` file whose cipher field says `cipher`.
fn openssh_key(cipher: &str) -> String {
    use base64ct::Encoding;
    let mut blob = b"openssh-key-v1\0".to_vec();
    blob.extend((cipher.len() as u32).to_be_bytes());
    blob.extend(cipher.as_bytes());
    blob.extend(4u32.to_be_bytes());
    blob.extend(b"none");
    let body = base64ct::Base64::encode_string(&blob);
    format!("-----BEGIN OPENSSH PRIVATE KEY-----\n{body}\n-----END OPENSSH PRIVATE KEY-----\n")
}

#[test]
fn the_default_keys_are_found_and_the_locked_one_is_named_not_offered() {
    let home = tempfile::tempdir().unwrap();
    let ssh = home.path().join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    // ssh's own default names: the encrypted ed25519 key comes first
    // in ssh's order, the readable rsa key after it.
    std::fs::write(ssh.join("id_ed25519"), openssh_key("aes256-ctr")).unwrap();
    std::fs::write(
        ssh.join("id_rsa"),
        "-----BEGIN RSA PRIVATE KEY-----\nMIIB\n-----END RSA PRIVATE KEY-----\n",
    )
    .unwrap();
    std::fs::write(ssh.join("id_rsa.pub"), "ssh-rsa AAAA\n").unwrap();
    std::env::set_var("HOME", home.path());
    std::env::set_var("USERPROFILE", home.path());
    std::env::remove_var("SSH_AUTH_SOCK");

    // No agent at all is one of the three sentences, not "error
    // authenticating".
    let agent = ssh_auth::probe_agent();
    assert_eq!(agent, Agent::Missing);
    assert!(!agent.usable());
    assert!(agent
        .sentence("github.com")
        .contains("no ssh agent is running"));

    let settings = ssh_config::for_host("github.com");
    let chain = ssh_auth::chain_for(
        "github.com",
        Some("git"),
        &settings,
        HostKind::Background,
        &agent,
        false,
    );
    assert_eq!(chain.user, "git");
    assert_eq!(
        chain.candidates,
        vec![SshCandidate::Key {
            path: ssh.join("id_rsa"),
            public: Some(ssh.join("id_rsa.pub")),
            passphrase: None,
        }],
        "the locked key must not be handed to libssh2, the readable one must"
    );
    let locked = ssh.join("id_ed25519").display().to_string();
    assert!(
        chain
            .notes
            .iter()
            .any(|note| note.contains(&locked) && note.ends_with("passphrase needed, skipped")),
        "{:?}",
        chain.notes
    );

    // On Windows the same directory yields no key at all, and says so
    // by name for the OpenSSH one.
    let windows = ssh_auth::chain_for(
        "github.com",
        Some("git"),
        &settings,
        HostKind::Background,
        &agent,
        true,
    );
    assert_eq!(
        windows.candidates,
        vec![SshCandidate::Key {
            path: ssh.join("id_rsa"),
            public: Some(ssh.join("id_rsa.pub")),
            passphrase: None,
        }],
        "the classic PKCS#1 key is the one Windows can read"
    );
    assert!(
        windows.notes.iter().any(|note| note
            .contains("this key file is in the OpenSSH format, which joy cannot read on Windows")),
        "{:?}",
        windows.notes
    );

    // An ssh config that names its own key file replaces the defaults.
    std::fs::write(
        ssh.join("config"),
        format!(
            "Host github.com\n  User git\n  IdentityFile {}\n",
            ssh.join("id_rsa").display()
        ),
    )
    .unwrap();
    let named = ssh_config::for_host("github.com");
    assert_eq!(named.identity_files, vec![ssh.join("id_rsa")]);
}
