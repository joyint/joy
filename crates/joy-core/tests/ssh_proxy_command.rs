// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! A host that is only reachable through a jump is refused BY NAME
//! (forge connection NG, design D1.4).
//!
//! libssh2 never opens a socket - the string "proxy" does not occur
//! anywhere in its sources - and libgit2 opens a plain TCP connection
//! to the host in the URL. A `ProxyCommand` host therefore failed with
//! a DNS or connect error that named the wrong cause, and a person had
//! no way to tell "your jump rule is not supported" from "the forge is
//! down". joy reads the ssh config and says which it is.
//!
//! Its own test binary: it moves HOME, which is process state.

use joy_core::vcs::forge::Auth;
use joy_core::vcs::{contact, ssh_config, HostKind};

#[test]
fn a_jump_host_is_refused_by_name_and_never_looked_up() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".ssh")).unwrap();
    std::fs::write(
        home.path().join(".ssh").join("config"),
        "Host jump.example.invalid\n  ProxyCommand /usr/bin/corkscrew proxy 8080 %h %p\n\
         \nHost hop.example.invalid\n  ProxyJump bastion.example.invalid\n\
         \nHost plain.example.invalid\n  User deploy\n\
         \nHost *\n  ServerAliveInterval 60\n\
         \nMatch exec \"nc -z vpn.corp.invalid 22\"\n  ProxyJump gateway.corp.invalid\n",
    )
    .unwrap();
    std::env::set_var("HOME", home.path());
    std::env::set_var("USERPROFILE", home.path());

    let sentence = ssh_config::refusal_for_url("git@jump.example.invalid:o/r.git")
        .expect("the jump host is refused");
    assert_eq!(
        sentence,
        "This host uses ProxyCommand in your ssh config. joy cannot run a proxy helper; \
         use the https remote for this host or remove the rule."
    );
    assert!(
        ssh_config::refusal_for_url("git@hop.example.invalid:o/r.git")
            .expect("the jump host is refused")
            .contains("ProxyJump")
    );
    // A host without a rule is not refused, and neither is https.
    assert!(ssh_config::refusal_for_url("git@plain.example.invalid:o/r.git").is_none());
    assert!(ssh_config::refusal_for_url("https://jump.example.invalid/o/r.git").is_none());
    // The `Match exec` block at the end of the file carries a
    // ProxyJump, and ssh2-config does not know the `Match` keyword, so
    // without joy's own scoping that rule would belong to the `Host *`
    // block above it and joy would refuse EVERY ssh remote on this
    // machine (design D1.4: refused by name, for the host that carries
    // the rule).
    assert!(ssh_config::refusal_for_url("git@github.com:o/r.git").is_none());
    assert!(ssh_config::refusal_for_url("git@plain.example.invalid:o/r.git").is_none());

    // And the contact itself stops before any lookup: the error is the
    // sentence, not a name resolution failure. A refusal that opens no
    // socket also spends no turn of the host's gap, so two of them in
    // a row do not sit out a throttle wait for a contact that never
    // happened (design D1.4).
    contact::set_gaps("jump.example.invalid=5000,default=0");
    let dest = tempfile::tempdir().unwrap();
    let started = std::time::Instant::now();
    let failed = forge_clone("git@jump.example.invalid:o/r.git", &dest.path().join("c"));
    let again = forge_clone("git@jump.example.invalid:o/r.git", &dest.path().join("d"));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "a refusal that never opens a socket must not wait out the host's gap"
    );
    assert!(again.contains("ProxyCommand"), "{again}");
    assert!(failed.contains("ProxyCommand"), "{failed}");
    for wrong in ["resolve", "dns", "getaddrinfo", "connect"] {
        assert!(
            !failed.to_ascii_lowercase().contains(wrong),
            "the refusal must not read like a network failure: {failed}"
        );
    }
}

fn forge_clone(url: &str, dest: &std::path::Path) -> String {
    joy_core::vcs::forge::clone(
        url,
        &Auth::local(HostKind::Background),
        dest,
        joy_core::vcs::forge::CLONE_DEPTH_FULL,
        &mut |_| true,
    )
    .expect_err("a jump host cannot be cloned")
    .to_string()
}
