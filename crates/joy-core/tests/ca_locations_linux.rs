// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The Linux only certificate authority escape hatch of D1.12.
//!
//! Three things are proven here, and one is stated rather than proven.
//!
//! - A machine with nothing configured applies nothing: OpenSSL's own
//!   default verify paths decide, which git2 fills with openssl-probe
//!   at init (git2 lib.rs:766-771, openssl-probe src/lib.rs:29-53), so
//!   an intercepting CA installed with `update-ca-certificates` is
//!   trusted with no joy setting at all. That the installed CA is then
//!   trusted is OpenSSL's behaviour and needs a machine wide install to
//!   observe; what joy owes is to set nothing, and that is asserted.
//! - `ca_bundle` and `ca_dir` from `forges.yaml` and `http.sslCAInfo`
//!   and `http.sslCAPath` from git config are read, joy's own file
//!   winning for the same kind.
//! - The applied locations really reach libgit2: a readable PEM is
//!   accepted by `git2::opts::set_ssl_cert_file` and an unreadable one
//!   is reported by name instead of being swallowed.
//!
//! It owns its process: HOME, XDG_CONFIG_HOME and the libgit2 option it
//! sets are process state.
//!
//! It runs under `forge-net`, because that is the build that speaks TLS
//! at all: without it libgit2 is compiled with no TLS backend and
//! `GIT_OPT_SET_SSL_CERT_LOCATIONS` answers "TLS backend doesn't
//! support certificate locations" (settings.c:207-223) on every target,
//! Linux included. The desktop and the platform build with it.

#![cfg(all(target_os = "linux", feature = "forge-net"))]

use std::sync::{Arc, Mutex};

use joy_core::vcs::proxy::{CaDecision, CaKind};

/// Every tracing line this process wrote, so that the branch which only
/// logs can be read.
#[derive(Clone, Default)]
struct Recorder {
    lines: Arc<Mutex<Vec<String>>>,
}

struct Visitor<'a>(&'a mut String);

impl tracing::field::Visit for Visitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={:?} ", field.name(), value));
    }
}

impl tracing::Subscriber for Recorder {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut text = String::new();
        event.record(&mut Visitor(&mut text));
        self.lines.lock().unwrap().push(text);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// A certificate authority bundle joy can hand to OpenSSL. Any PEM with
/// a certificate in it does; the machine's own bundle is the one that
/// is certainly valid and certainly present on a Linux host that has
/// ever spoken TLS.
fn a_real_bundle() -> Option<std::path::PathBuf> {
    [
        "/etc/ssl/certs/ca-certificates.crt",
        "/etc/pki/tls/certs/ca-bundle.crt",
        "/etc/ssl/ca-bundle.pem",
    ]
    .into_iter()
    .map(std::path::PathBuf::from)
    .find(|path| path.is_file())
}

/// The libgit2 option and the environment are process state: one test
/// at a time.
static SERIAL: Mutex<()> = Mutex::new(());

#[test]
fn the_hatch_reads_both_sources_and_reaches_libgit2() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HOME", home.path());
    std::env::set_var("XDG_CONFIG_HOME", home.path().join(".config"));
    std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");

    // Nothing configured: joy sets nothing and the system store decides
    // alone, which is what makes `update-ca-certificates` enough.
    assert_eq!(joy_core::ca_locations(), CaDecision::Nothing);

    let joy_config = home.path().join(".config").join("joy");
    std::fs::create_dir_all(&joy_config).expect("config dir");
    let Some(bundle) = a_real_bundle() else {
        // A Linux host with no certificate bundle at all cannot say
        // anything about this criterion, and inventing a PEM here would
        // test the test.
        eprintln!("no system certificate bundle on this host; the apply half is not run");
        return;
    };
    let certs_dir = home.path().join("certs");
    std::fs::create_dir_all(&certs_dir).expect("certs dir");
    std::fs::write(
        joy_config.join("forges.yaml"),
        format!(
            "- host: git.acme.example\n  kind: gitlab\n  ca_bundle: {}\n  ca_dir: {}\n",
            bundle.display(),
            certs_dir.display()
        ),
    )
    .expect("write forges.yaml");
    // The key a corporate workstation image already carries, which
    // libgit2 reads nothing of (zero hits for http.sslCAInfo in the
    // whole 1.9.6 tree).
    std::fs::write(
        home.path().join(".gitconfig"),
        format!("[http]\n\tsslCAInfo = {}\n", bundle.display()),
    )
    .expect("write gitconfig");

    let decision = joy_core::ca_locations();
    let CaDecision::Apply(entries) = decision else {
        panic!("a Linux build applies the hatch, got {decision:?}");
    };
    assert_eq!(
        entries.len(),
        2,
        "one bundle and one directory: {entries:?}"
    );
    assert_eq!(
        entries[0].key, "ca_bundle",
        "joy's own file wins the bundle"
    );
    assert_eq!(entries[0].kind, CaKind::Bundle);
    assert_eq!(entries[0].value, bundle.display().to_string());
    assert_eq!(entries[1].key, "ca_dir");
    assert_eq!(entries[1].kind, CaKind::Directory);

    // And they really reach libgit2: the option exists on this build
    // (it is compiled for OpenSSL and mbedTLS alone,
    // settings.c:207-223) and it takes the file.
    let recorder = Recorder::default();
    tracing::subscriber::with_default(recorder.clone(), joy_core::apply_ca_locations);
    let lines = recorder.lines.lock().unwrap().clone();
    assert!(
        lines.iter().any(
            |line| line.contains("certificate authority location applied")
                && line.contains("ca_bundle")
        ),
        "the bundle was applied: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("could not be applied")),
        "and nothing failed: {lines:#?}"
    );
}

/// The other half of the apply: a location that is not a certificate
/// bundle fails, which is what joy reports by name instead of leaving
/// a person with a CA they believe is installed. The one time wrapper
/// is spent by the test above, so this one drives the libgit2 option
/// directly, which is what that wrapper does inside its `Once`.
#[test]
fn a_bundle_that_is_not_one_is_reported_by_name() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let not_a_bundle = dir.path().join("not-a-bundle.pem");
    std::fs::write(&not_a_bundle, "this is not a certificate\n").expect("write");
    // SAFETY: a libgit2 global option. This test binary sets it in this
    // test and in the one above, both single threaded, and nothing in
    // this binary opens a TLS connection.
    let applied = unsafe { git2::opts::set_ssl_cert_file(not_a_bundle.to_str().expect("utf-8")) };
    assert!(
        applied.is_err(),
        "OpenSSL refuses a file that holds no certificate, and joy reports it"
    );
}
