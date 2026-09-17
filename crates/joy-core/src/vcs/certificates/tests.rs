// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::path::Path;

use super::*;

/// github.com's ed25519 host key and the fingerprint GitHub publishes.
const GITHUB_ED25519: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
const GITHUB_ED25519_FINGERPRINT: &str = "SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU";
/// Another ed25519 key (codeberg.org's), for the changed-key cases.
const OTHER_ED25519: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIIVIC02vnjFyL+I4RHfvIGNtOgJMe769VTF1VR4EB3ZB";

/// The trust question is process-wide, like the passphrase question, so
/// the tests that install one run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn key(base64: &str) -> Vec<u8> {
    use base64ct::Encoding;
    base64ct::Base64::decode_vec(base64).expect("a base64 key")
}

/// A machine whose known_hosts files are the ones this test wrote.
fn settings(files: Vec<PathBuf>, strict: StrictHostKeys, hashed: bool) -> HostSettings {
    HostSettings {
        user_known_hosts: files,
        global_known_hosts: Vec::new(),
        strict_host_keys: strict,
        hash_known_hosts: hashed,
        ..HostSettings::default()
    }
}

fn wrote(dir: &Path, text: &str) -> PathBuf {
    let path = dir.join("known_hosts");
    std::fs::write(&path, text).unwrap();
    path
}

/// The state and the sentence a refusal really produces, read the way
/// the contact boundary reads them: the code survives libgit2's
/// overwrite, the sentence travels in the cell.
fn refusal(result: Result<Status, git2::Error>) -> (git2::ErrorCode, String) {
    // `CertificateCheckStatus` has no Debug, so the Ok side cannot be
    // unwrapped by `expect_err`
    let Err(error) = result else {
        panic!("the key was accepted where a refusal was expected");
    };
    let sentence = take_refusal().expect("joy's own sentence");
    (error.code(), sentence)
}

/// The error alone, for a test that reads the code and the sentence
/// apart from each other.
fn refused(result: Result<Status, git2::Error>) -> git2::Error {
    let Err(error) = result else {
        panic!("the key was accepted where a refusal was expected");
    };
    error
}

#[test]
fn a_known_host_is_accepted_and_nothing_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let file = wrote(
        dir.path(),
        &format!("github.com ssh-ed25519 {GITHUB_ED25519}\n"),
    );
    let before = std::fs::read_to_string(&file).unwrap();
    for kind in [
        HostKind::Interactive,
        HostKind::Background,
        HostKind::Delegated,
    ] {
        let mut trust = Trust::at(
            kind,
            "github.com",
            settings(vec![file.clone()], StrictHostKeys::Ask, true),
            22,
        );
        assert!(matches!(
            trust.host_key(
                "github.com",
                Some(SshHostKeyType::Ed255219),
                Some(&key(GITHUB_ED25519))
            ),
            Ok(Status::CertificateOk)
        ));
    }
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
}

/// The J4h acceptance: a `Background` host refuses with the state J5
/// defines and prints the line to paste.
#[test]
fn a_background_host_refuses_an_unknown_host_and_prints_the_line_to_paste() {
    let dir = tempfile::tempdir().unwrap();
    let file = wrote(dir.path(), "");
    for kind in [HostKind::Background, HostKind::Delegated] {
        let mut trust = Trust::at(
            kind,
            "github.com",
            settings(vec![file.clone()], StrictHostKeys::Ask, true),
            22,
        );
        let (code, sentence) = refusal(trust.host_key(
            "github.com",
            Some(SshHostKeyType::Ed255219),
            Some(&key(GITHUB_ED25519)),
        ));
        assert_eq!(code, git2::ErrorCode::Certificate, "{kind}");
        for part in [
            "github.com",
            "port 22",
            "ssh-ed25519",
            GITHUB_ED25519_FINGERPRINT,
            &file.display().to_string(),
            &format!("github.com ssh-ed25519 {GITHUB_ED25519}"),
        ] {
            assert!(
                sentence.contains(part),
                "{kind}: {part} missing in {sentence}"
            );
        }
        // never asked, never written
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
    }
}

/// The same refusal as the contact boundary reads it: the state is the
/// one J5 defined, and the sentence a person sees is joy's, not
/// libgit2's "invalid or unknown remote ssh hostkey".
#[test]
fn the_refusal_reaches_the_person_as_needs_host_trust_with_joys_own_sentence() {
    use crate::vcs::contact::{ContactDirection, ContactEvidence, Failure};
    let dir = tempfile::tempdir().unwrap();
    let file = wrote(dir.path(), "");
    let mut trust = Trust::at(
        HostKind::Background,
        "github.com",
        settings(vec![file], StrictHostKeys::Ask, true),
        22,
    );
    let refused = refused(trust.host_key(
        "github.com",
        Some(SshHostKeyType::Ed255219),
        Some(&key(GITHUB_ED25519)),
    ));
    // what libgit2 hands back: joy's code, libgit2's own message
    // (ssh_libssh2.c:765) and its own class
    let libgit2 = git2::Error::new(
        refused.code(),
        git2::ErrorClass::Ssh,
        "invalid or unknown remote ssh hostkey",
    );
    let evidence = ContactEvidence::new(
        libgit2,
        "git@github.com:joyint/joy.git",
        ContactDirection::Fetch,
        crate::vcs::contact::CredentialSource::AgentPresented,
    );
    let verdict = crate::vcs::contact::verdict(&evidence);
    assert_eq!(verdict.failure, Failure::NeedsHostTrust);
    assert!(
        verdict.sentence.contains(GITHUB_ED25519_FINGERPRINT),
        "the fingerprint is in the sentence: {}",
        verdict.sentence
    );
    // libgit2's own words stay on the detail line
    assert!(verdict
        .detail
        .contains("invalid or unknown remote ssh hostkey"));
}

/// The J4h acceptance: a changed key is refused in all three host
/// kinds, with both fingerprints and the file and the line number.
#[test]
fn a_changed_key_is_refused_in_every_host_kind() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // even a host that would say yes to anything is not asked
    set_trust_prompt(|_| true);
    let dir = tempfile::tempdir().unwrap();
    let file = wrote(
        dir.path(),
        &format!("other.example.com ssh-ed25519 {OTHER_ED25519}\ngithub.com ssh-ed25519 {OTHER_ED25519}\n"),
    );
    let before = std::fs::read_to_string(&file).unwrap();
    for kind in [
        HostKind::Interactive,
        HostKind::Background,
        HostKind::Delegated,
    ] {
        let mut trust = Trust::at(
            kind,
            "github.com",
            settings(vec![file.clone()], StrictHostKeys::Ask, true),
            22,
        );
        let (code, sentence) = refusal(trust.host_key(
            "github.com",
            Some(SshHostKeyType::Ed255219),
            Some(&key(GITHUB_ED25519)),
        ));
        assert_eq!(code, git2::ErrorCode::Certificate, "{kind}");
        for part in [
            GITHUB_ED25519_FINGERPRINT,
            &known_hosts::fingerprint(&key(OTHER_ED25519)),
            &file.display().to_string(),
            "line 2",
        ] {
            assert!(
                sentence.contains(part),
                "{kind}: {part} missing in {sentence}"
            );
        }
    }
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    clear_trust_prompt();
}

/// The J4h acceptance: a fresh machine is offered the fingerprint,
/// accepting writes ONE hashed line, and the next run is silent.
#[test]
fn an_interactive_host_is_offered_the_fingerprint_and_the_yes_is_written_once() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let asked: Arc<Mutex<Vec<TrustRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&asked);
    set_trust_prompt(move |request| {
        seen.lock().unwrap().push(request.clone());
        true
    });
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(".ssh").join("known_hosts");
    let site = || {
        Trust::at(
            HostKind::Interactive,
            "github.com",
            settings(vec![file.clone()], StrictHostKeys::Ask, true),
            22,
        )
    };
    assert!(matches!(
        site().host_key(
            "github.com",
            Some(SshHostKeyType::Ed255219),
            Some(&key(GITHUB_ED25519))
        ),
        Ok(Status::CertificateOk)
    ));
    let request = asked
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("one question");
    assert_eq!(request.host, "github.com");
    assert_eq!(request.port, 22);
    assert_eq!(request.key_type, "ssh-ed25519");
    assert_eq!(request.fingerprint, GITHUB_ED25519_FINGERPRINT);
    assert!(request.published.is_none(), "no pin is consulted yet");
    // one hashed line, because the person's config says so
    let written = std::fs::read_to_string(&file).unwrap();
    assert_eq!(written.lines().count(), 1);
    assert!(written.starts_with("|1|"), "{written}");
    // and the next run is silent: no question, no second line
    assert!(matches!(
        site().host_key(
            "github.com",
            Some(SshHostKeyType::Ed255219),
            Some(&key(GITHUB_ED25519))
        ),
        Ok(Status::CertificateOk)
    ));
    assert_eq!(asked.lock().unwrap().len(), 1);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), written);
    clear_trust_prompt();
}

#[test]
fn strict_host_key_checking_decides_whether_the_person_is_asked() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let questions = Arc::new(Mutex::new(0usize));
    let counted = Arc::clone(&questions);
    set_trust_prompt(move |_| {
        *counted.lock().unwrap() += 1;
        true
    });
    // `yes`: refuse without asking, and say why
    let dir = tempfile::tempdir().unwrap();
    let strict_file = dir.path().join("strict").join("known_hosts");
    let mut strict = Trust::at(
        HostKind::Interactive,
        "github.com",
        settings(vec![strict_file.clone()], StrictHostKeys::Yes, false),
        22,
    );
    let (_, sentence) = refusal(strict.host_key(
        "github.com",
        Some(SshHostKeyType::Ed255219),
        Some(&key(GITHUB_ED25519)),
    ));
    assert!(sentence.contains("StrictHostKeyChecking yes"), "{sentence}");
    assert!(!strict_file.exists());
    assert_eq!(*questions.lock().unwrap(), 0);
    // `accept-new`: add it without asking
    let new_file = dir.path().join("new").join("known_hosts");
    let mut accept = Trust::at(
        HostKind::Interactive,
        "github.com",
        settings(vec![new_file.clone()], StrictHostKeys::AcceptNew, false),
        22,
    );
    assert!(matches!(
        accept.host_key(
            "github.com",
            Some(SshHostKeyType::Ed255219),
            Some(&key(GITHUB_ED25519))
        ),
        Ok(Status::CertificateOk)
    ));
    assert_eq!(*questions.lock().unwrap(), 0);
    assert_eq!(
        std::fs::read_to_string(&new_file).unwrap(),
        format!("github.com ssh-ed25519 {GITHUB_ED25519}\n")
    );
    // `accept-new` still writes nothing where nobody is watching
    let background_file = dir.path().join("background").join("known_hosts");
    let mut background = Trust::at(
        HostKind::Background,
        "github.com",
        settings(
            vec![background_file.clone()],
            StrictHostKeys::AcceptNew,
            false,
        ),
        22,
    );
    refused(background.host_key(
        "github.com",
        Some(SshHostKeyType::Ed255219),
        Some(&key(GITHUB_ED25519)),
    ));
    let _ = take_refusal();
    assert!(!background_file.exists());
    clear_trust_prompt();
}

#[test]
fn a_no_that_the_person_gave_is_a_refusal_and_not_a_write() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    set_trust_prompt(|_| false);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("known_hosts");
    let mut trust = Trust::at(
        HostKind::Interactive,
        "github.com",
        settings(vec![file.clone()], StrictHostKeys::Ask, false),
        22,
    );
    let (code, sentence) = refusal(trust.host_key(
        "github.com",
        Some(SshHostKeyType::Ed255219),
        Some(&key(GITHUB_ED25519)),
    ));
    assert_eq!(code, git2::ErrorCode::Certificate);
    assert!(sentence.contains("not trusted"), "{sentence}");
    // a no is not a dead end either: the line stays in reach
    assert!(
        sentence.contains(&format!("github.com ssh-ed25519 {GITHUB_ED25519}")),
        "{sentence}"
    );
    assert!(sentence.contains(&file.display().to_string()), "{sentence}");
    assert!(!file.exists());
    clear_trust_prompt();
}

/// The path a person really hits today: nothing in this tree installs a
/// question yet, because joy-cli's call sites are package J6's and its
/// acceptance is that no ssh contact fails without a way to accept. So
/// an `Interactive` host refuses here, and D1.8b's rule holds on that
/// path too: the sentence says why nobody was asked and names the one
/// next step, the file and the line to paste.
#[test]
fn an_interactive_host_with_no_question_installed_still_names_the_next_step() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    clear_trust_prompt();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("known_hosts");
    let mut trust = Trust::at(
        HostKind::Interactive,
        "github.com",
        settings(vec![file.clone()], StrictHostKeys::Ask, true),
        22,
    );
    let (code, sentence) = refusal(trust.host_key(
        "github.com",
        Some(SshHostKeyType::Ed255219),
        Some(&key(GITHUB_ED25519)),
    ));
    assert_eq!(code, git2::ErrorCode::Certificate);
    for part in [
        "github.com",
        "port 22",
        "ssh-ed25519",
        GITHUB_ED25519_FINGERPRINT,
        "no way to put the question to you",
        &file.display().to_string(),
        &format!("github.com ssh-ed25519 {GITHUB_ED25519}"),
    ] {
        assert!(sentence.contains(part), "{part} missing in {sentence}");
    }
    // nothing was written, and the hashed form is what a yes WOULD
    // have written, not what a person is asked to paste
    assert!(!file.exists());
}

#[test]
fn a_host_known_for_another_type_says_so_in_the_question() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let asked: Arc<Mutex<Vec<TrustRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&asked);
    set_trust_prompt(move |request| {
        seen.lock().unwrap().push(request.clone());
        false
    });
    let dir = tempfile::tempdir().unwrap();
    let file = wrote(
        dir.path(),
        &format!("github.com ecdsa-sha2-nistp256 {OTHER_ED25519}\n"),
    );
    let mut trust = Trust::at(
        HostKind::Interactive,
        "github.com",
        settings(vec![file], StrictHostKeys::Ask, false),
        22,
    );
    let (_, sentence) = refusal(trust.host_key(
        "github.com",
        Some(SshHostKeyType::Ed255219),
        Some(&key(GITHUB_ED25519)),
    ));
    assert!(sentence.contains("not trusted"), "{sentence}");
    let request = asked
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("one question");
    assert_eq!(request.other_types, vec!["ecdsa-sha2-nistp256".to_string()]);
    clear_trust_prompt();
}

#[test]
fn a_revoked_key_is_refused_in_every_host_kind() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    set_trust_prompt(|_| true);
    let dir = tempfile::tempdir().unwrap();
    let file = wrote(
        dir.path(),
        &format!("@revoked github.com ssh-ed25519 {GITHUB_ED25519}\n"),
    );
    for kind in [
        HostKind::Interactive,
        HostKind::Background,
        HostKind::Delegated,
    ] {
        let mut trust = Trust::at(
            kind,
            "github.com",
            settings(vec![file.clone()], StrictHostKeys::AcceptNew, false),
            22,
        );
        let (code, sentence) = refusal(trust.host_key(
            "github.com",
            Some(SshHostKeyType::Ed255219),
            Some(&key(GITHUB_ED25519)),
        ));
        assert_eq!(code, git2::ErrorCode::Certificate, "{kind}");
        assert!(sentence.contains("@revoked"), "{kind}: {sentence}");
        assert!(sentence.contains("line 1"), "{kind}: {sentence}");
    }
    clear_trust_prompt();
}

#[test]
fn a_certificate_host_key_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let file = wrote(dir.path(), "");
    let mut certificate = Vec::new();
    let name = b"ssh-ed25519-cert-v01@openssh.com";
    certificate.extend((name.len() as u32).to_be_bytes());
    certificate.extend(name);
    certificate.extend(b"the rest of a host certificate");
    let mut trust = Trust::at(
        HostKind::Interactive,
        "git.example.com",
        settings(vec![file], StrictHostKeys::AcceptNew, false),
        22,
    );
    // git2 reports a certificate host key as `Unknown` with no name
    let (code, sentence) = refusal(trust.host_key(
        "git.example.com",
        Some(SshHostKeyType::Unknown),
        Some(&certificate),
    ));
    assert_eq!(code, git2::ErrorCode::Certificate);
    assert!(
        sentence.contains("joy cannot verify certificate host keys yet"),
        "{sentence}"
    );
    assert!(
        sentence.contains("ssh-ed25519-cert-v01@openssh.com"),
        "{sentence}"
    );
}

/// The pin file is data nobody consults yet (decision 23), so the
/// branch is driven with the recorded pins to prove it is right before
/// the decision switches it on.
#[test]
fn a_pin_answers_only_where_no_file_knows_the_host() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let asked: Arc<Mutex<Vec<TrustRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&asked);
    set_trust_prompt(move |request| {
        seen.lock().unwrap().push(request.clone());
        false
    });
    let dir = tempfile::tempdir().unwrap();
    let empty = wrote(dir.path(), "");
    // `Background` accepts the pinned key and writes nothing
    let mut background = Trust::at(
        HostKind::Background,
        "github.com",
        settings(vec![empty.clone()], StrictHostKeys::Ask, true),
        22,
    )
    .reading_pins();
    assert!(matches!(
        background.host_key(
            "github.com",
            Some(SshHostKeyType::Ed255219),
            Some(&key(GITHUB_ED25519))
        ),
        Ok(Status::CertificateOk)
    ));
    assert_eq!(std::fs::read_to_string(&empty).unwrap(), "");
    // `Interactive` is still asked, and the question carries the page
    let mut interactive = Trust::at(
        HostKind::Interactive,
        "github.com",
        settings(vec![empty.clone()], StrictHostKeys::Ask, true),
        22,
    )
    .reading_pins();
    refused(interactive.host_key(
        "github.com",
        Some(SshHostKeyType::Ed255219),
        Some(&key(GITHUB_ED25519)),
    ));
    let _ = take_refusal();
    let request = asked
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("one question");
    let published = request.published.expect("the pin names the page");
    assert!(published.contains("GitHub"), "{published}");
    assert!(
        published.contains("https://docs.github.com/"),
        "{published}"
    );
    // a file that knows the host beats the pin, in both directions
    let known = wrote(
        dir.path(),
        &format!("github.com ssh-ed25519 {OTHER_ED25519}\n"),
    );
    let mut beaten = Trust::at(
        HostKind::Background,
        "github.com",
        settings(vec![known], StrictHostKeys::Ask, true),
        22,
    )
    .reading_pins();
    let (_, sentence) = refusal(beaten.host_key(
        "github.com",
        Some(SshHostKeyType::Ed255219),
        Some(&key(GITHUB_ED25519)),
    ));
    assert!(sentence.contains("changed"), "{sentence}");
    clear_trust_prompt();
}

#[test]
fn a_pinned_host_that_presents_another_key_names_the_joy_version_and_the_page() {
    let dir = tempfile::tempdir().unwrap();
    let empty = wrote(dir.path(), "");
    for kind in [
        HostKind::Interactive,
        HostKind::Background,
        HostKind::Delegated,
    ] {
        let mut trust = Trust::at(
            kind,
            "github.com",
            settings(vec![empty.clone()], StrictHostKeys::AcceptNew, false),
            22,
        )
        .reading_pins();
        let (code, sentence) = refusal(trust.host_key(
            "github.com",
            Some(SshHostKeyType::Ed255219),
            Some(&key(OTHER_ED25519)),
        ));
        assert_eq!(code, git2::ErrorCode::Certificate, "{kind}");
        for part in [
            "GitHub",
            env!("CARGO_PKG_VERSION"),
            GITHUB_ED25519_FINGERPRINT,
            "https://docs.github.com/",
        ] {
            assert!(
                sentence.contains(part),
                "{kind}: {part} missing in {sentence}"
            );
        }
        assert_eq!(std::fs::read_to_string(&empty).unwrap(), "");
    }
}

// ---- the https branch -------------------------------------------------

/// One DER element: tag, length (short or long form), contents.
fn element(tag: u8, contents: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    if contents.len() < 0x80 {
        out.push(contents.len() as u8);
    } else {
        let length = (contents.len() as u32).to_be_bytes();
        let bytes: Vec<u8> = length.iter().copied().skip_while(|b| *b == 0).collect();
        out.push(0x80 | bytes.len() as u8);
        out.extend(bytes);
    }
    out.extend(contents);
    out
}

/// `Name ::= SEQUENCE OF SET OF AttributeTypeAndValue`, with the one
/// attribute a detail line needs.
fn name(common: &str) -> Vec<u8> {
    let attribute = element(
        0x30,
        &[
            element(0x06, &[0x55, 0x04, 0x03]),
            element(0x0c, common.as_bytes()),
        ]
        .concat(),
    );
    element(0x30, &element(0x31, &attribute))
}

fn certificate(issuer: &str, subject: &str) -> Vec<u8> {
    let tbs = element(
        0x30,
        &[
            element(0xa0, &element(0x02, &[2])), // [0] version v3
            element(0x02, &[0x13, 0x37]),        // serialNumber
            element(0x30, &element(0x06, &[0x2a, 0x86, 0x48])), // signature
            name(issuer),
            element(0x30, &element(0x17, b"260917000000Z")), // validity
            name(subject),
        ]
        .concat(),
    );
    element(
        0x30,
        &[tbs, element(0x30, &[]), element(0x03, &[0, 1])].concat(),
    )
}

#[test]
fn the_x509_branch_decides_nothing_and_names_the_issuer() {
    forget_refusal();
    let mut trust = Trust::new(HostKind::Background, None);
    let der = certificate("Acme Corporate Root CA", "github.com");
    assert!(matches!(
        trust.x509(&der),
        Ok(Status::CertificatePassthrough)
    ));
    let note = x509_note().expect("the branch stashed what it read");
    assert_eq!(note.issuer.as_deref(), Some("Acme Corporate Root CA"));
    assert_eq!(note.subject.as_deref(), Some("github.com"));
    // it never refuses and never writes a sentence: the verdict is
    // libgit2's (D1.8c)
    assert_eq!(take_refusal(), None);
    // and a certificate joy cannot read is still libgit2's business
    let mut broken = Trust::new(HostKind::Background, None);
    assert!(matches!(
        broken.x509(b"not a certificate"),
        Ok(Status::CertificatePassthrough)
    ));
    assert_eq!(x509_note(), Some(X509Note::default()));
}

/// What package J4p writes into this branch: the state and the
/// sentence (D1.8c). The branch decides nothing, so the state comes
/// from the classifier reading the error the operation returned, and
/// the issuer this branch stashed is what the detail line puts in
/// front of libgit2's own text.
#[test]
fn the_issuer_this_branch_stashed_reaches_the_tls_untrusted_detail_line() {
    forget_refusal();
    let mut trust = Trust::new(HostKind::Background, None);
    let der = certificate("Acme Corporate Root CA", "github.com");
    assert!(matches!(
        trust.x509(&der),
        Ok(Status::CertificatePassthrough)
    ));
    // The three texts the three TLS backends produce for a chain they
    // do not trust: openssl.c:381-384, stransport.c:117-120 and
    // winhttp.c:718-740. The state has to be the same on all three.
    let cases = [
        (
            git2::Error::new(
                git2::ErrorCode::Certificate,
                git2::ErrorClass::Ssl,
                "the SSL certificate is invalid",
            ),
            "the SSL certificate is invalid",
        ),
        (
            git2::Error::new(
                git2::ErrorCode::Certificate,
                git2::ErrorClass::Ssl,
                "untrusted connection error",
            ),
            "untrusted connection error",
        ),
        (
            git2::Error::new(
                git2::ErrorCode::GenericError,
                git2::ErrorClass::Http,
                "SSL certificate signed by unknown CA",
            ),
            "SSL certificate signed by unknown CA",
        ),
    ];
    for (error, text) in cases {
        let evidence = super::super::contact::ContactEvidence::new(
            error,
            "https://github.com/joyint/joy.git",
            super::super::contact::ContactDirection::Fetch,
            super::super::contact::CredentialSource::TokenPresented,
        );
        let verdict = super::super::contact::verdict(&evidence);
        assert_eq!(
            verdict.failure,
            super::super::contact::Failure::TlsUntrusted,
            "{text}"
        );
        assert_eq!(
            verdict.sentence,
            "The certificate for github.com is not trusted by this machine's certificate store."
        );
        assert_eq!(
            verdict.detail,
            format!("issued by 'Acme Corporate Root CA', chain not trusted; libgit2: {text}"),
            "the issuer in front of libgit2's own words"
        );
        // one next step, and the instruction behind it is the one this
        // operating system's store needs
        assert_eq!(verdict.next_step.as_deref(), Some("show what to do"));
        assert!(
            verdict.guidance.is_some(),
            "the per OS certificate instruction of D1.8c"
        );
    }
    // A failure that is not the certificate keeps libgit2's line alone,
    // even while a certificate was seen on this contact.
    let evidence = super::super::contact::ContactEvidence::new(
        git2::Error::new(
            git2::ErrorCode::GenericError,
            git2::ErrorClass::Net,
            "unexpected http status code: 404",
        ),
        "https://github.com/joyint/joy.git",
        super::super::contact::ContactDirection::Fetch,
        super::super::contact::CredentialSource::TokenPresented,
    );
    assert_eq!(
        super::super::contact::verdict(&evidence).detail,
        "libgit2: unexpected http status code: 404"
    );
    forget_refusal();
}

/// The J4h acceptance: an https contact goes through the SAME closure.
/// It needs the network and a real forge, which is why it is ignored by
/// default; `cargo test -p joy-core --features forge-net -- --ignored`
/// runs it.
#[cfg(feature = "forge-net")]
#[test]
#[ignore = "contacts github.com over https"]
fn an_https_contact_goes_through_the_same_closure() {
    forget_refusal();
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.certificate_check(check(HostKind::Background, None));
    let mut remote = git2::Remote::create_detached("https://github.com/joyint/joy.git").unwrap();
    let connection = remote
        .connect_auth(git2::Direction::Fetch, Some(callbacks), None)
        .expect("github.com answers");
    drop(connection);
    let note = x509_note().expect("the one closure saw the https certificate");
    assert_eq!(note.subject.as_deref(), Some("github.com"));
    assert!(
        note.issuer.is_some(),
        "the issuer is named for the detail line"
    );
    // the branch accepted nothing and refused nothing
    assert_eq!(take_refusal(), None);
}

/// The J4h acceptance, against the real github.com: an empty
/// known_hosts offers the fingerprint GitHub publishes, accepting
/// writes ONE hashed line, and the next contact is silent. It needs the
/// network, so it is ignored by default;
/// `cargo test -p joy-core --features forge-net -- --ignored` runs it.
#[cfg(feature = "forge-net")]
#[test]
#[ignore = "contacts github.com over ssh"]
fn a_fresh_machine_is_offered_the_fingerprint_github_publishes() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let asked: Arc<Mutex<Vec<TrustRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&asked);
    set_trust_prompt(move |request| {
        seen.lock().unwrap().push(request.clone());
        true
    });
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(".ssh").join("known_hosts");
    let contact = || {
        let mut callbacks = git2::RemoteCallbacks::new();
        callbacks.certificate_check(
            Trust::at(
                HostKind::Interactive,
                "github.com",
                settings(vec![file.clone()], StrictHostKeys::Ask, true),
                22,
            )
            .into_check(),
        );
        let mut remote =
            git2::Remote::create_detached("ssh://git@github.com/joyint/joy.git").unwrap();
        // No credentials callback: the host key is checked before any
        // credential is asked for (ssh_libssh2.c:830), so the contact
        // ends in "authentication required but no callback set" AFTER
        // this closure has had its say.
        let error = remote
            .connect_auth(git2::Direction::Fetch, Some(callbacks), None)
            .err()
            .expect("no credential was offered");
        assert_eq!(
            error.code(),
            git2::ErrorCode::Auth,
            "the host key was accepted and the login is what failed: {error}"
        );
    };
    contact();
    let request = asked
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("one question");
    assert_eq!(request.host, "github.com");
    assert_eq!(request.port, 22);
    // whichever type libssh2 negotiated, the fingerprint is one GitHub
    // publishes
    let published = pins::published_for("github.com").expect("github.com is parked");
    assert!(
        published
            .keys
            .iter()
            .any(|key| key.key_type == request.key_type && key.fingerprint == request.fingerprint),
        "{} {} is not a key GitHub publishes",
        request.key_type,
        request.fingerprint
    );
    let written = std::fs::read_to_string(&file).unwrap();
    assert_eq!(written.lines().count(), 1, "one line: {written}");
    assert!(written.starts_with("|1|"), "hashed: {written}");
    // the next run is silent
    contact();
    assert_eq!(asked.lock().unwrap().len(), 1, "asked twice");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), written);
    clear_trust_prompt();
}
