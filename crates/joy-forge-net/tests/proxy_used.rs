// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! A configured proxy really carries the connector's contacts, and
//! `NO_PROXY` really takes a host out again (D1.11, D2.8).
//!
//! Its own test binary, because it sets environment variables and a
//! process has only one environment.
//!
//! The "proxy" is the in process fake, and what it records is the
//! `CONNECT` the client sends it. That is the whole of what a test can
//! see without building a real tunnel, and it is the right thing to
//! look at: an HTTP proxy always gets `CONNECT` here, exactly as
//! libgit2 does it for the engine (it "always speaks HTTP CONNECT",
//! httpclient.c:686-700), so both halves of joy behave alike.

use joy_forge_net::fake::{FakeForge, Reply};
use joy_forge_net::gitconfig::GitConfig;
use joy_forge_net::http::Http;
use joy_forge_net::trust::Trust;

#[test]
fn a_configured_proxy_carries_the_contact_and_no_proxy_takes_a_host_out() {
    let fake = FakeForge::start(|_| Reply::json(200, "{}"));
    std::env::set_var("http_proxy", fake.base());
    std::env::set_var("NO_PROXY", "direct.example, other.example");

    let http = Http::new(Trust::Platform, GitConfig::from_text(""), "joy-forge/test");
    // The fake answers the CONNECT and tunnels nothing, so the call
    // itself fails; what it proves is where the contact went.
    let _ = http.get("http://forge.example/api/v3/user").call();
    let seen = fake.calls();
    assert_eq!(seen.len(), 1, "the proxy was asked exactly once: {seen:?}");
    assert_eq!(seen[0].method, "CONNECT");
    assert_eq!(seen[0].path, "forge.example:80");

    // A host NO_PROXY names is contacted directly, so the proxy sees
    // nothing more. The entries are trimmed, which is the correction
    // D1.11 makes against libgit2: without it `b.com` in
    // "a.com, b.com" is silently lost and would go through the proxy.
    let direct = http.get("http://direct.example/api/v3/user").call();
    assert!(direct.is_err(), "the contact went direct and found nothing");
    assert_eq!(
        fake.calls().len(),
        1,
        "the proxy saw no second request: {:?}",
        fake.calls()
    );
    let other = http.get("http://other.example/api/v3/user").call();
    assert!(other.is_err());
    assert_eq!(fake.calls().len(), 1);

    std::env::remove_var("http_proxy");
    std::env::remove_var("NO_PROXY");
}
