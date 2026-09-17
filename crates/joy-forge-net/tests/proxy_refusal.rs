// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The SOCKS refusal, at the client (D1.11, applied to the connector by
//! D2.8: "the plugin refuses a SOCKS proxy with the same sentence").
//!
//! Its own test binary, because it sets an environment variable and a
//! process has only one environment. The request is never attempted:
//! libgit2 would parse a SOCKS URL as an HTTP proxy and speak HTTP
//! CONNECT to it, so the engine refuses by name rather than failing
//! obscurely, and the connector says the same sentence.

use joy_forge_net::gitconfig::GitConfig;
use joy_forge_net::http::{Http, HttpError};
use joy_forge_net::trust::Trust;

#[test]
fn a_socks_proxy_is_refused_by_name_and_no_request_is_made() {
    std::env::set_var("ALL_PROXY", "socks5://user:s3cr3t@socks.example:1080");
    let http = Http::new(Trust::Platform, GitConfig::from_text(""), "joy-forge/test");
    // The address is unreachable on purpose: the refusal has to come
    // before anything is attempted.
    let error = http
        .get("https://127.0.0.1:1/user")
        .call()
        .expect_err("a SOCKS proxy is refused");
    let HttpError::Refused { message } = error else {
        panic!("the refusal must be joy's own, not a transport error: {error}");
    };
    assert_eq!(
        message,
        "joy cannot use the SOCKS proxy socks5://socks.example:1080; it supports HTTP and HTTPS proxies only."
    );
    assert!(!message.contains("s3cr3t"), "no proxy password: {message}");
    std::env::remove_var("ALL_PROXY");
}
