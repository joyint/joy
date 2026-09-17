// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The proxy decision of D1.11 and the trust store hatch of D1.12.
//!
//! The decision is a function of its inputs here, never of the process
//! environment: [`super::decide`] takes the environment and the
//! credential lookup as parameters, so these tests decide nothing for
//! each other and nothing for the rest of the crate. The one test that
//! really sets `HTTPS_PROXY` is the end to end one in
//! tests/proxy_basic_auth.rs, which owns its own process.

use super::*;

/// A git config written for one test, opened as libgit2 sees it.
fn written_config(lines: &str) -> (tempfile::TempDir, git2::Config) {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("config");
    std::fs::write(&file, lines).expect("write config");
    let config = git2::Config::open(&file).expect("open config");
    (dir, config)
}

/// Nobody answers for a proxy credential.
fn no_credential(_: &ProxyUrl) -> Option<(String, String)> {
    None
}

fn decide_with(
    url: &str,
    config: Option<&git2::Config>,
    remote: Option<&str>,
    env: &Environment,
) -> Result<Proxy, Unsupported> {
    decide(url, config, remote, env, &mut no_credential)
}

fn env_with(https: Option<&str>, no: Option<&str>) -> Environment {
    Environment {
        https_proxy: https.map(str::to_string),
        http_proxy: None,
        all_proxy: None,
        no_proxy: no.map(str::to_string),
    }
}

// ---- NO_PROXY ----------------------------------------------------------

/// The acceptance criterion of J4p, as a matcher test: libgit2 compares
/// the bytes between two commas as they stand, so the space in
/// `"a.com, b.com"` becomes part of the name and `b.com` silently goes
/// through the proxy. joy trims.
#[test]
fn no_proxy_entries_are_trimmed() {
    assert!(
        no_proxy_matches("b.com", 443, "a.com, b.com"),
        "the entry after the comma counts, space or no space"
    );
    assert!(no_proxy_matches("a.com", 443, "a.com, b.com"));
    assert!(!no_proxy_matches("c.com", 443, "a.com, b.com"));
    assert!(
        no_proxy_matches("b.com", 443, "  a.com ,\tb.com\n"),
        "every kind of blank around an entry"
    );
}

#[test]
fn no_proxy_grammar_is_libgit2s() {
    // a lone star is every host (net.c:1074-1075)
    assert!(no_proxy_matches("anything.example", 443, "*"));
    // *.domain matches the domain itself and one below it
    assert!(no_proxy_matches("acme.example", 443, "*.acme.example"));
    assert!(no_proxy_matches("git.acme.example", 443, "*.acme.example"));
    // .domain is the same rule
    assert!(no_proxy_matches("git.acme.example", 443, ".acme.example"));
    // and neither matches a name that merely ends in those letters
    assert!(!no_proxy_matches("notacme.example", 443, "*.acme.example"));
    // no wildcard means an exact name
    assert!(no_proxy_matches("acme.example", 443, "acme.example"));
    assert!(!no_proxy_matches("git.acme.example", 443, "acme.example"));
    // host names are case insensitive
    assert!(no_proxy_matches("ACME.example", 443, "acme.EXAMPLE"));
    // a port in the pattern must match (net.c:1100-1103)
    assert!(no_proxy_matches("acme.example", 8443, "acme.example:8443"));
    assert!(!no_proxy_matches("acme.example", 443, "acme.example:8443"));
    // and there is no CIDR in this grammar: the text is a name
    assert!(!no_proxy_matches("10.0.0.7", 443, "10.0.0.0/8"));
    assert!(no_proxy_matches("10.0.0.7", 443, "10.0.0.7"));
    // an empty entry matches nothing, which is what an empty list is
    assert!(!no_proxy_matches("acme.example", 443, ""));
    assert!(!no_proxy_matches("acme.example", 443, ",,"));
}

/// A pattern whose port is not a port names a port no contact can have,
/// and matches nothing. libgit2 compares the port TEXT
/// (net.c:1100-1103), so `acme.example:99999` matches no contact there
/// either; reading it as "this entry names no port" would bypass the
/// proxy for that host on EVERY port, which is the opposite of what the
/// person wrote.
#[test]
fn a_port_that_is_not_a_port_matches_nothing() {
    assert!(!no_proxy_matches("acme.example", 443, "acme.example:99999"));
    assert!(!no_proxy_matches("acme.example", 99, "acme.example:99999"));
    // and the rest of the list is still read
    assert!(no_proxy_matches("b.com", 443, "acme.example:99999, b.com"));
    // a leading zero is still a port, because it parses
    assert!(no_proxy_matches("acme.example", 443, "acme.example:0443"));
}

/// One matcher for the whole product (JOY-02A3-E4): the engine's
/// evaluation IS the connector's, so a NO_PROXY the person wrote cannot
/// mean one thing to a git contact and another to a REST call. The
/// corpus is the grammar's corners plus the port that is not a port,
/// which is where the two had drifted apart.
///
/// Every row carries the answer D1.11 requires, because the identity
/// alone would hold just as well if both sides were wrong together;
/// the identity is asserted after it, as the cheap guard against a
/// second copy growing somewhere.
#[test]
fn the_engine_and_the_connector_share_one_matcher() {
    for (host, port, list, expected) in [
        ("acme.example", 443u16, "*", true),
        // `*.domain` and `.domain` cover the domain itself
        ("acme.example", 443, "*.acme.example", true),
        ("git.acme.example", 443, "*.acme.example", true),
        ("git.acme.example", 443, ".acme.example", true),
        // and nothing that merely ends in those letters
        ("notacme.example", 443, "*.acme.example", false),
        // a port no contact can have matches nothing, on no port
        ("acme.example", 443, "acme.example:99999", false),
        ("acme.example", 99, "acme.example:99999", false),
        // and the rest of the list still counts
        ("b.com", 443, "acme.example:99999, b.com", true),
        // a pattern's port must match when it names one
        ("acme.example", 8443, "acme.example:8443", true),
        ("acme.example", 443, "acme.example:8443", false),
        // entries are trimmed, which libgit2 does not do
        ("b.com", 443, "a.com, b.com", true),
        // no CIDR in this grammar
        ("10.0.0.7", 443, "10.0.0.0/8", false),
        // an IPv6 literal is compared bare, bracket or not
        ("::1", 8443, "[::1]", true),
        ("::1", 8443, "[::1]:8443", true),
        ("::1", 443, "[::1]:8443", false),
        ("acme.example", 443, ",,", false),
    ] {
        assert_eq!(
            no_proxy_matches(host, port, list),
            expected,
            "{host}:{port} against {list:?}"
        );
        assert_eq!(
            no_proxy_matches(host, port, list),
            joy_forge_net::proxy::no_proxy_matches(host, port, list),
            "two matchers again: {host}:{port} against {list:?}"
        );
    }
}

/// The half libgit2 does not do: `http_proxy_config` never looks at
/// no_proxy (remote.c:1085-1133), so a proxy from git config was used
/// for a host the person excluded. joy applies the list to every
/// source.
#[test]
fn no_proxy_applies_to_a_config_sourced_proxy_too() {
    let (_dir, config) = written_config("[http]\n\tproxy = http://proxy.acme.example:8080\n");
    let proxy = decide_with(
        "https://b.com/o/r.git",
        Some(&config),
        None,
        &env_with(None, Some("a.com, b.com")),
    )
    .expect("a bypass is not a refusal");
    assert_eq!(proxy.outcome(), Outcome::Bypassed);
    assert_eq!(proxy.name(), None);
    // and the same repository still reaches the proxy for another host
    let proxy = decide_with(
        "https://c.com/o/r.git",
        Some(&config),
        None,
        &env_with(None, Some("a.com, b.com")),
    )
    .expect("no refusal");
    assert_eq!(proxy.outcome(), Outcome::Specified);
    assert_eq!(proxy.name(), Some("proxy.acme.example:8080"));
}

/// A NO_PROXY hit is `GIT_PROXY_NONE` even when joy itself found no
/// proxy: under `GIT_PROXY_AUTO` libgit2 would walk its own order and
/// use a config proxy the person excluded.
#[test]
fn a_bypassed_host_never_falls_through_to_auto() {
    let proxy = decide_with(
        "https://b.com/o/r.git",
        None,
        None,
        &env_with(None, Some("b.com")),
    )
    .expect("no refusal");
    assert_eq!(proxy.outcome(), Outcome::Bypassed);
}

// ---- the three outcomes -----------------------------------------------

#[test]
fn nothing_configured_leaves_the_walk_to_libgit2() {
    let proxy = decide_with(
        "https://github.com/o/r.git",
        None,
        None,
        &Environment::default(),
    )
    .expect("no refusal");
    assert_eq!(proxy.outcome(), Outcome::Auto);
    assert_eq!(proxy.name(), None);
}

#[test]
fn a_known_proxy_is_always_specified() {
    let proxy = decide_with(
        "https://github.com/o/r.git",
        None,
        None,
        &env_with(Some("http://proxy.acme.example:8080"), None),
    )
    .expect("no refusal");
    // `GIT_PROXY_SPECIFIED` whenever a proxy is known (D1.11): under
    // AUTO, WinHTTP hands a NULL URL to `acquire_credentials` on the
    // 407 path (winhttp.c:1255-1270).
    assert_eq!(proxy.outcome(), Outcome::Specified);
    assert_eq!(proxy.name(), Some("proxy.acme.example:8080"));
}

/// `ALL_PROXY` is git's variable and libgit2 reads it nowhere
/// (remote.c:1137-1160 knows http_proxy and https_proxy only), so joy
/// passes it as a specified URL.
#[test]
fn all_proxy_reaches_libgit2_only_because_joy_names_it() {
    let env = Environment {
        https_proxy: None,
        http_proxy: None,
        all_proxy: Some("http://proxy.acme.example:3128".to_string()),
        no_proxy: None,
    };
    let proxy = decide_with("https://github.com/o/r.git", None, None, &env).expect("no refusal");
    assert_eq!(proxy.outcome(), Outcome::Specified);
    assert_eq!(proxy.name(), Some("proxy.acme.example:3128"));
    // and the per scheme variable wins over it, as in git
    let env = Environment {
        https_proxy: Some("http://per-scheme.example:8080".to_string()),
        ..env
    };
    let proxy = decide_with("https://github.com/o/r.git", None, None, &env).expect("no refusal");
    assert_eq!(proxy.name(), Some("per-scheme.example:8080"));
}

/// A proxy with no scheme is an http proxy, as
/// `git_net_url_parse_http` reads it, and its default port is 80.
#[test]
fn a_proxy_without_a_scheme_is_http() {
    let proxy = decide_with(
        "https://github.com/o/r.git",
        None,
        None,
        &env_with(Some("proxy.acme.example"), None),
    )
    .expect("no refusal");
    assert_eq!(proxy.name(), Some("proxy.acme.example:80"));
}

// ---- the config order --------------------------------------------------

#[test]
fn the_config_order_is_libgit2s() {
    let (_dir, config) = written_config(
        "[remote \"origin\"]\n\tproxy = http://per-remote.example:1\n\
         [http \"https://github.com/o/r.git\"]\n\tproxy = http://per-url.example:2\n\
         [http \"https://github.com\"]\n\tproxy = http://per-host.example:3\n\
         [http]\n\tproxy = http://general.example:4\n",
    );
    let env = env_with(Some("http://env.example:5"), None);
    // remote.<name>.proxy first (remote.c:1105-1111)
    let proxy = decide_with(
        "https://github.com/o/r.git",
        Some(&config),
        Some("origin"),
        &env,
    )
    .expect("no refusal");
    assert_eq!(proxy.name(), Some("per-remote.example:1"));
    // then the most specific http.<url>.proxy
    let proxy =
        decide_with("https://github.com/o/r.git", Some(&config), None, &env).expect("no refusal");
    assert_eq!(proxy.name(), Some("per-url.example:2"));
    // then the same key walked down to the bare host
    let proxy = decide_with(
        "https://github.com/other/thing.git",
        Some(&config),
        None,
        &env,
    )
    .expect("no refusal");
    assert_eq!(proxy.name(), Some("per-host.example:3"));
    // and the environment only when no config key answered
    let (_bare_dir, bare) = written_config("");
    let proxy =
        decide_with("https://github.com/o/r.git", Some(&bare), None, &env).expect("no refusal");
    assert_eq!(proxy.name(), Some("env.example:5"));
}

#[test]
fn a_non_default_port_is_part_of_the_config_key() {
    let (_dir, config) = written_config(
        "[http \"https://git.acme.example:8443/o/r.git\"]\n\tproxy = http://proxy.example:8080\n",
    );
    let proxy = decide_with(
        "https://git.acme.example:8443/o/r.git",
        Some(&config),
        None,
        &Environment::default(),
    )
    .expect("no refusal");
    assert_eq!(proxy.name(), Some("proxy.example:8080"));
}

/// git's own way of switching a proxy off for one repository: an entry
/// that exists and is empty ends the search (remote.c:1051-1068,
/// http.c:336), so the environment does not take over.
#[test]
fn an_empty_config_value_turns_the_proxy_off() {
    let (_dir, config) = written_config("[http]\n\tproxy = \n");
    let proxy = decide_with(
        "https://github.com/o/r.git",
        Some(&config),
        None,
        &env_with(Some("http://env.example:8080"), None),
    )
    .expect("no refusal");
    assert_eq!(proxy.outcome(), Outcome::Bypassed);
}

/// An ssh remote takes no HTTP proxy, and a local one takes nothing at
/// all: saying so here keeps the ssh chain's failures free of a proxy
/// that never applied.
#[test]
fn only_http_transports_take_a_proxy() {
    let env = env_with(Some("http://proxy.acme.example:8080"), None);
    for url in [
        "git@github.com:o/r.git",
        "ssh://git@github.com/o/r.git",
        "/home/someone/repo",
    ] {
        let proxy = decide_with(url, None, None, &env).expect("no refusal");
        assert_eq!(proxy.outcome(), Outcome::Bypassed, "{url}");
    }
}

// ---- SOCKS -------------------------------------------------------------

/// libgit2 speaks HTTP CONNECT to whatever it is given
/// (httpclient.c:686-700), so a SOCKS proxy is refused before a socket
/// is opened, by name.
#[test]
fn a_socks_proxy_is_refused_by_name() {
    let refusal = decide_with(
        "https://github.com/o/r.git",
        None,
        None,
        &env_with(Some("socks5://127.0.0.1:1080"), None),
    )
    .expect_err("joy cannot speak SOCKS");
    assert_eq!(
        refusal.sentence,
        "joy cannot use the SOCKS proxy socks5://127.0.0.1:1080; it supports HTTP and HTTPS \
         proxies only."
    );
    // every SOCKS spelling, and the credential never rides in the text
    for value in ["socks://h:1", "socks4://h:1", "socks5h://user:pw@h:1"] {
        let refusal = decide_with(
            "https://github.com/o/r.git",
            None,
            None,
            &env_with(Some(value), None),
        )
        .expect_err("refused");
        assert!(refusal.sentence.contains("SOCKS"), "{}", refusal.sentence);
        assert!(!refusal.sentence.contains("pw"), "{}", refusal.sentence);
    }
}

#[test]
fn another_scheme_joy_does_not_speak_is_refused_too() {
    let refusal = decide_with(
        "https://github.com/o/r.git",
        None,
        None,
        &env_with(Some("ftp://proxy.example:21"), None),
    )
    .expect_err("refused");
    assert_eq!(
        refusal.sentence,
        "joy cannot use the ftp proxy ftp://proxy.example:21; it supports HTTP and HTTPS \
         proxies only."
    );
}

// ---- credentials -------------------------------------------------------

/// The credential goes into the URL, because git2 hardwires
/// `credentials: None` in `ProxyOptions::raw` (proxy_options.rs:44-53)
/// and libgit2 presents a proxy URL's own userinfo before it consults
/// any callback (http.c:141-152). The NAME never carries it.
#[test]
fn a_helper_credential_rides_in_the_url_and_never_in_the_name() {
    let mut asked: Vec<String> = Vec::new();
    let mut credential = |proxy: &ProxyUrl| {
        asked.push(proxy.credential_url());
        Some(("picard".to_string(), "the:pass@word".to_string()))
    };
    let proxy = decide(
        "https://github.com/o/r.git",
        None,
        None,
        &env_with(Some("http://proxy.acme.example:8080"), None),
        &mut credential,
    )
    .expect("no refusal");
    assert_eq!(asked, vec!["http://proxy.acme.example:8080".to_string()]);
    assert_eq!(proxy.name(), Some("proxy.acme.example:8080"));
    // percent encoded, because libgit2 decodes the userinfo it parses
    // (net.c:401-409) and a `:` or an `@` would otherwise be read as a
    // delimiter
    assert_eq!(
        proxy.url.as_deref(),
        Some("http://picard:the%3Apass%40word@proxy.acme.example:8080")
    );
    // and nothing a person reads carries it
    let printed = format!("{proxy:?}");
    assert!(!printed.contains("the:pass"), "{printed}");
    assert!(!printed.contains("%3Apass"), "{printed}");
    assert!(printed.contains("proxy.acme.example:8080"), "{printed}");
}

/// The helper is asked with `protocol=http` for an `https://` proxy
/// too (D1.11). It is one login, to one machine in the middle: a person
/// who stored it once must not have to store it a second time because
/// the proxy URL gained an `s`.
#[test]
fn an_https_proxy_is_looked_up_under_protocol_http() {
    let mut asked: Vec<String> = Vec::new();
    let mut credential = |proxy: &ProxyUrl| {
        asked.push(proxy.credential_url());
        None
    };
    let proxy = decide(
        "https://github.com/o/r.git",
        None,
        None,
        &env_with(Some("https://proxy.acme.example:8443"), None),
        &mut credential,
    )
    .expect("no refusal");
    assert_eq!(asked, vec!["http://proxy.acme.example:8443".to_string()]);
    // and the proxy itself is still dialled over https
    assert_eq!(
        proxy.url.as_deref(),
        Some("https://proxy.acme.example:8443")
    );
    assert_eq!(proxy.name(), Some("proxy.acme.example:8443"));
}

/// What the person wrote into their own configuration is used as
/// written, and no helper is asked.
#[test]
fn a_userinfo_in_the_configured_url_is_kept() {
    let mut asked = 0usize;
    let mut credential = |_: &ProxyUrl| {
        asked += 1;
        None
    };
    let proxy = decide(
        "https://github.com/o/r.git",
        None,
        None,
        &env_with(Some("http://picard:secret@proxy.acme.example:8080"), None),
        &mut credential,
    )
    .expect("no refusal");
    assert_eq!(asked, 0, "the URL already carried both halves");
    assert_eq!(proxy.name(), Some("proxy.acme.example:8080"));
    assert_eq!(
        proxy.url.as_deref(),
        Some("http://picard:secret@proxy.acme.example:8080")
    );
}

/// git's shape for "the password is in the helper": a user name in the
/// URL and nothing else (git-config http.proxy).
#[test]
fn a_user_name_without_a_password_asks_the_helper_for_that_user() {
    let mut seen: Option<String> = None;
    let mut credential = |proxy: &ProxyUrl| {
        seen = proxy.user.clone();
        Some(("picard".to_string(), "secret".to_string()))
    };
    let proxy = decide(
        "https://github.com/o/r.git",
        None,
        None,
        &env_with(Some("http://picard@proxy.acme.example:8080"), None),
        &mut credential,
    )
    .expect("no refusal");
    assert_eq!(seen.as_deref(), Some("picard"));
    assert_eq!(
        proxy.url.as_deref(),
        Some("http://picard:secret@proxy.acme.example:8080")
    );
}

/// No helper answered: the proxy is still used, without a credential,
/// and the 407 that follows becomes `proxy_auth` with the proxy's own
/// name (D1.8c).
#[test]
fn a_proxy_without_a_credential_is_still_the_proxy() {
    let proxy = decide_with(
        "https://github.com/o/r.git",
        None,
        None,
        &env_with(Some("http://proxy.acme.example:8080"), None),
    )
    .expect("no refusal");
    assert_eq!(proxy.outcome(), Outcome::Specified);
    assert_eq!(
        proxy.url.as_deref(),
        Some("http://proxy.acme.example:8080"),
        "no userinfo at all, not an empty one"
    );
}

#[test]
fn a_proxy_url_is_redacted_for_every_text_a_person_reads() {
    assert_eq!(
        redacted("http://picard:secret@proxy.acme.example:8080"),
        "http://<credential>@proxy.acme.example:8080"
    );
    assert_eq!(
        redacted("socks5://127.0.0.1:1080"),
        "socks5://127.0.0.1:1080"
    );
    assert_eq!(
        redacted("proxy.acme.example:8080"),
        "proxy.acme.example:8080"
    );
}

/// libgit2 has one message that echoes the proxy URL joy built,
/// userinfo included: `git_error_set(GIT_ERROR_HTTP, "invalid URL:
/// '%s'", proxy)` (http.c:340-342). Whatever a text that reaches a
/// person came from, the credential is taken out of it.
#[test]
fn a_libgit2_message_that_echoes_the_proxy_url_loses_the_credential() {
    assert_eq!(
        scrubbed("invalid URL: 'http://picard:secret@proxy.acme.example:8080'"),
        "invalid URL: 'http://<credential>@proxy.acme.example:8080'"
    );
    // the whole authority, and not one character past it
    assert_eq!(
        scrubbed("failed http://picard:s@e%40cret@proxy.acme:8080/path@x now"),
        "failed http://<credential>@proxy.acme:8080/path@x now"
    );
    // a message with no URL in it is handed back as it stands
    let plain = "the SSL certificate is invalid";
    assert_eq!(scrubbed(plain), plain);
    // and so is a URL that carries no userinfo
    let clean = "failed to connect to https://github.com/o/r.git";
    assert_eq!(scrubbed(clean), clean);
    // an ssh remote's user name is a user name, not a credential, but
    // this runs only where a credentialed proxy was configured, so the
    // safe reading wins
    assert_eq!(
        scrubbed("ssh://git@github.com/o/r"),
        "ssh://<credential>@github.com/o/r"
    );
}

// ---- the Linux only CA escape hatch (D1.12) ----------------------------

fn bundle(key: &str, source: &str, value: &str) -> CaEntry {
    CaEntry {
        key: key.to_string(),
        source: source.to_string(),
        value: value.to_string(),
        kind: CaKind::Bundle,
    }
}

#[test]
fn nothing_configured_is_nothing_applied() {
    assert_eq!(
        ca_decision(Vec::new(), TrustStore::OpenSsl),
        CaDecision::Nothing
    );
}

/// On an OpenSSL build the hatch applies, one entry per kind: the two
/// settings are one libgit2 option with two arguments
/// (settings.c:207-223).
#[test]
fn the_hatch_applies_on_an_openssl_build() {
    let entries = vec![
        bundle("ca_bundle", "forges.yaml", "/etc/acme/ca.pem"),
        bundle("http.sslCAInfo", "git config", "/etc/other/ca.pem"),
        CaEntry {
            key: "ca_dir".to_string(),
            source: "forges.yaml".to_string(),
            value: "/etc/acme/certs".to_string(),
            kind: CaKind::Directory,
        },
    ];
    let CaDecision::Apply(applied) = ca_decision(entries, TrustStore::OpenSsl) else {
        panic!("an OpenSSL build applies the hatch");
    };
    // joy's own file wins over whatever the workstation image left in
    // git config, and one bundle is applied, not two
    assert_eq!(applied.len(), 2);
    assert_eq!(applied[0].key, "ca_bundle");
    assert_eq!(applied[0].value, "/etc/acme/ca.pem");
    assert_eq!(applied[1].key, "ca_dir");
}

/// `GIT_OPT_SET_SSL_CERT_LOCATIONS` is compiled for OpenSSL and mbedTLS
/// alone, so on the other two the entry is refused BY NAME, with the
/// store that decides instead. Both sentences are asserted here,
/// because neither can be run here.
#[test]
fn the_hatch_is_refused_by_name_on_macos_and_windows() {
    let entries = vec![bundle(
        "ca_bundle",
        "/home/picard/.config/joy/forges.yaml",
        "/etc/acme/ca.pem",
    )];
    let CaDecision::Refused(sentences) = ca_decision(entries.clone(), TrustStore::Keychain) else {
        panic!("macOS refuses it");
    };
    assert_eq!(
        sentences,
        vec![
            "joy ignores ca_bundle from /home/picard/.config/joy/forges.yaml: it does not apply \
             here, because this system checks certificates against its own store. To trust an \
             internal CA, add your organisation's CA to the login or System keychain and mark it \
             trusted."
                .to_string()
        ]
    );
    let CaDecision::Refused(sentences) = ca_decision(entries, TrustStore::WindowsStore) else {
        panic!("Windows refuses it");
    };
    assert!(
        sentences[0].ends_with(
            "To trust an internal CA, your administrator must install the CA in the Windows \
             certificate store."
        ),
        "{}",
        sentences[0]
    );
}

#[test]
fn forges_yaml_gives_up_its_two_ca_keys_and_nothing_else() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("forges.yaml");
    std::fs::write(
        &file,
        "- host: git.acme.example\n  kind: gitlab\n  api_base: https://git.acme.example/api/v4\n\
         - host: git2.acme.example\n  kind: gitea\n  ca_bundle: /etc/acme/ca.pem\n  \
         ca_dir: /etc/acme/certs\n  ca_bundle_note: ignored\n",
    )
    .expect("write");
    let entries = forges_yaml_ca(&file);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].key, "ca_bundle");
    assert_eq!(entries[0].value, "/etc/acme/ca.pem");
    assert_eq!(entries[0].kind, CaKind::Bundle);
    assert_eq!(entries[1].key, "ca_dir");
    assert_eq!(entries[1].kind, CaKind::Directory);
    assert_eq!(entries[0].source, file.display().to_string());
}

#[test]
fn a_missing_or_unreadable_forges_yaml_is_not_a_failure() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(forges_yaml_ca(&dir.path().join("forges.yaml")).is_empty());
    let broken = dir.path().join("broken.yaml");
    std::fs::write(&broken, "this: is not a list\n").expect("write");
    assert!(forges_yaml_ca(&broken).is_empty());
}

/// The one named exception of D1.12: libgit2 reads neither key (no hit
/// for either in the whole 1.9.6 tree), so joy reads them for it, on
/// Linux.
#[test]
fn the_two_git_config_keys_are_read() {
    let (_dir, config) = written_config(
        "[http]\n\tsslCAInfo = /etc/acme/ca.pem\n\tsslCAPath = /etc/acme/certs\n\tproxy = \n",
    );
    let entries = git_config_ca(&config);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].key, "http.sslCAInfo");
    assert_eq!(entries[0].kind, CaKind::Bundle);
    assert_eq!(entries[1].key, "http.sslCAPath");
    assert_eq!(entries[1].kind, CaKind::Directory);
    assert_eq!(entries[0].source, "git config");
    // and an empty value is not a location
    let (_dir, empty) = written_config("[http]\n\tsslCAInfo = \n");
    assert!(git_config_ca(&empty).is_empty());
}
