// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The Linux only certificate authority escape hatch of D1.12.
//!
//! Four things are proven here, and one is stated rather than proven.
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
//! - A location libgit2 will not take is REPORTED by name, with the key
//!   it came from and the file it names, instead of leaving a person
//!   with a CA they believe is installed.
//! - The applied locations really reach libgit2: a readable PEM is
//!   accepted by `git2::opts::set_ssl_cert_file`.
//!
//! Only the last of those needs a TLS backend, so only it runs under
//! `forge-net`: without that feature libgit2 is compiled with no TLS
//! backend and `GIT_OPT_SET_SSL_CERT_LOCATIONS` answers "TLS backend
//! doesn't support certificate locations" (settings.c:207-223) on every
//! target, Linux included. The three criteria a person's day depends on
//! run in the default suite, because a criterion nobody runs is not
//! met.
//!
//! The file owns its process: HOME, XDG_CONFIG_HOME and the libgit2
//! option it sets are process state.

#![cfg(target_os = "linux")]

use std::sync::{Arc, Mutex};

use joy_core::vcs::proxy::{CaDecision, CaEntry, CaKind};

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
#[cfg(feature = "forge-net")]
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

/// A HOME with nothing in it, which is what "no joy setting" means.
fn an_empty_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HOME", home.path());
    std::env::set_var("XDG_CONFIG_HOME", home.path().join(".config"));
    std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    home
}

/// The acceptance criterion: an intercepting CA installed with
/// `update-ca-certificates` is trusted on Linux with NO joy setting.
/// What joy owes for that is to set nothing at all, so that OpenSSL's
/// default verify paths are what decides, and that is what this asserts.
#[test]
fn nothing_configured_is_nothing_applied_and_the_system_store_decides() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _home = an_empty_home();
    assert_eq!(joy_core::ca_locations(), CaDecision::Nothing);
    // and applying that decision touches no libgit2 option and says
    // nothing, because there is nothing to say
    let recorder = Recorder::default();
    tracing::subscriber::with_default(recorder.clone(), || {
        joy_core::apply_ca_decision(CaDecision::Nothing)
    });
    assert!(
        recorder.lines.lock().unwrap().is_empty(),
        "a machine with nothing configured hears nothing"
    );
}

/// Both sources are read, and joy's own file wins over whatever the
/// workstation image left in git config for the same kind. The paths
/// here name nothing that exists: this half is the READING, and it is
/// the same on every Linux host.
#[test]
fn the_hatch_reads_forges_yaml_and_the_two_git_config_keys() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let home = an_empty_home();
    let joy_config = home.path().join(".config").join("joy");
    std::fs::create_dir_all(&joy_config).expect("config dir");
    std::fs::write(
        joy_config.join("forges.yaml"),
        "- host: git.acme.example\n  kind: gitlab\n  ca_bundle: /etc/acme/ca.pem\n  \
         ca_dir: /etc/acme/certs\n",
    )
    .expect("write forges.yaml");
    // The key a corporate workstation image already carries, which
    // libgit2 reads nothing of (zero hits for http.sslCAInfo in the
    // whole 1.9.6 tree).
    std::fs::write(
        home.path().join(".gitconfig"),
        "[http]\n\tsslCAInfo = /etc/image/ca.pem\n\tsslCAPath = /etc/image/certs\n",
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
    assert_eq!(entries[0].value, "/etc/acme/ca.pem");
    assert_eq!(entries[1].key, "ca_dir");
    assert_eq!(entries[1].kind, CaKind::Directory);
    assert_eq!(entries[1].value, "/etc/acme/certs");
}

/// A location libgit2 will not take is reported BY NAME, which is what
/// keeps a person from believing a CA is installed when it is not.
///
/// This drives the applying half with the decision as a parameter,
/// because the one time wrapper around it can be spent only once per
/// process. Why libgit2 refuses differs with the build and does not
/// matter to the branch under test: under `forge-net` OpenSSL refuses a
/// file that holds no certificate, and without a TLS backend libgit2
/// refuses the option itself (settings.c:207-223).
#[test]
fn a_location_libgit2_will_not_take_is_reported_by_name() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let not_a_bundle = dir.path().join("not-a-bundle.pem");
    std::fs::write(&not_a_bundle, "this is not a certificate\n").expect("write");
    let decision = CaDecision::Apply(vec![CaEntry {
        key: "ca_bundle".to_string(),
        source: "/home/picard/.config/joy/forges.yaml".to_string(),
        value: not_a_bundle.display().to_string(),
        kind: CaKind::Bundle,
    }]);

    let recorder = Recorder::default();
    tracing::subscriber::with_default(recorder.clone(), || joy_core::apply_ca_decision(decision));
    let lines = recorder.lines.lock().unwrap().clone();
    let reported = lines
        .iter()
        .find(|line| line.contains("certificate authority location could not be applied"))
        .unwrap_or_else(|| panic!("the refusal is reported: {lines:#?}"));
    assert!(reported.contains("ca_bundle"), "the key: {reported}");
    assert!(
        reported.contains("forges.yaml"),
        "and where it came from: {reported}"
    );
    assert!(
        reported.contains(&not_a_bundle.display().to_string()),
        "and the location itself: {reported}"
    );
}

/// Every refused entry of a macOS or Windows machine is said out loud,
/// and no libgit2 option is touched for it.
#[test]
fn a_refused_entry_is_said_out_loud() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let recorder = Recorder::default();
    tracing::subscriber::with_default(recorder.clone(), || {
        joy_core::apply_ca_decision(CaDecision::Refused(vec![
            "joy ignores ca_bundle".to_string()
        ]))
    });
    let lines = recorder.lines.lock().unwrap().clone();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("joy ignores ca_bundle")),
        "{lines:#?}"
    );
}

/// And the locations really reach libgit2: the option exists on this
/// build (it is compiled for OpenSSL and mbedTLS alone,
/// settings.c:207-223) and it takes a real bundle.
#[test]
#[cfg(feature = "forge-net")]
fn an_applied_bundle_reaches_libgit2() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(bundle) = a_real_bundle() else {
        // A Linux host with no certificate bundle at all cannot say
        // anything about this criterion, and inventing a PEM here would
        // test the test.
        eprintln!("no system certificate bundle on this host; the apply half is not run");
        return;
    };
    let decision = CaDecision::Apply(vec![CaEntry {
        key: "ca_bundle".to_string(),
        source: "/home/picard/.config/joy/forges.yaml".to_string(),
        value: bundle.display().to_string(),
        kind: CaKind::Bundle,
    }]);

    let recorder = Recorder::default();
    tracing::subscriber::with_default(recorder.clone(), || joy_core::apply_ca_decision(decision));
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
