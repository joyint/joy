// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! THE `certificate_check` closure (design D1.4a, D1.8c, D1.12).
//!
//! git2 0.21 holds exactly ONE such slot per contact
//! (`certificate_check: Option<Box<CertificateCheck<'a>>>`,
//! remote_callbacks.rs:27), so exactly one module owns it and every
//! `RemoteCallbacks` joy builds installs the closure this module
//! returns ([`check`]). The closure dispatches on the certificate kind,
//! and the two kinds have OPPOSITE rules, which is the only way both
//! can hold:
//!
//! - **Host key (an ssh contact).** joy performs the whole check in
//!   Rust and answers `CertificateOk` or an error, never
//!   `CertificatePassthrough`: passthrough would hand the verdict back
//!   to libgit2's own check, which reads one file with `strcmp` and
//!   knows neither hashed entries, nor `UserKnownHostsFile`, nor
//!   `StrictHostKeyChecking` (see [`super::known_hosts`]).
//! - **x509 (an https contact).** This branch returns
//!   `CertificatePassthrough`, so libgit2's own verdict stands: joy
//!   never accepts a certificate libgit2 refused and never refuses one
//!   it accepted. It reads no verdict, because git2 0.21 drops
//!   libgit2's `valid` flag before the closure runs
//!   (remote_callbacks.rs:413-418); it only stashes the certificate's
//!   issuer and subject for the detail line of `tls_untrusted`, whose
//!   sentence and state are package J4p's work.
//!
//! **Why this module carries its own state.** git2 hands the closure
//! the host name and nothing else: no port, and no verdict. So joy
//! carries the port and the host kind itself, taken from the remote as
//! the person configured it, and resolves the person's ssh config for
//! the host libgit2 really dialled (`Host` aliases included, D1.4).
//!
//! **Why the refusal sentence travels in a cell.** libgit2 overwrites
//! whatever message the callback set: on a refusal it writes
//! "invalid or unknown remote ssh hostkey" over it and returns the
//! callback's code (ssh_libssh2.c:759-767). The CODE survives, which is
//! what the classifier reads (`needs_host_trust`, D1.8b), but the
//! sentence does not, so joy keeps its own sentence in a thread local
//! cell and the contact boundary substitutes it when the operation
//! returns ([`take_refusal`]).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use git2::cert::{Cert, SshHostKeyType};
use git2::CertificateCheckStatus as Status;

use super::known_hosts::{self, pins, Verdict};
use super::ssh_config::{HostSettings, StrictHostKeys};
use crate::host::HostKind;

/// What an `Interactive` host is asked about an unknown host key.
#[derive(Debug, Clone)]
pub struct TrustRequest {
    pub host: String,
    pub port: u16,
    /// The known_hosts spelling of the type (`ssh-ed25519`).
    pub key_type: String,
    /// `SHA256:` and the unpadded base64 the forges publish.
    pub fingerprint: String,
    /// The file the line would be added to.
    pub file: PathBuf,
    /// The line joy would add, already in the shape the person's config
    /// asks for (hashed when `HashKnownHosts yes`).
    pub line: String,
    /// "this is the key GitHub publishes at <url>", when a pin says so.
    pub published: Option<String>,
    /// The key types this host already has lines for, when it has any.
    /// A host with lines only for other types is not a new host, and
    /// the question has to say so.
    pub other_types: Vec<String>,
}

type Ask = Arc<dyn Fn(&TrustRequest) -> bool + Send + Sync>;

static PROMPT: Mutex<Option<Ask>> = Mutex::new(None);

/// Install the question joy asks before it trusts a host key.
///
/// Only a host where a person is sitting installs one: joy-core has no
/// terminal and no window of its own, so this is the seam through
/// which a front end lends joy-core its terminal. joy's own CLI
/// installs one in `cli_main`, and only for an `Interactive` host
/// (package J10, `joy-cli/src/lib.rs`); the desktop installs its own
/// for a foreground action. A host that installs none refuses an
/// unknown host key instead of trusting it, whatever its host kind
/// claims. That refusal names the file and the line to paste, so the
/// next step exists on that path too (D1.8b).
pub fn set_trust_prompt(ask: impl Fn(&TrustRequest) -> bool + Send + Sync + 'static) {
    *PROMPT.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(ask));
}

/// Take the question back. For a host that changes mode, and for tests.
pub fn clear_trust_prompt() {
    *PROMPT.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Whether this process installed one. A host proves with this that it
/// lends its terminal in the mode it meant to and in no other; nothing
/// in the engine reads it, because the engine asks and handles
/// "nobody was asked" as its own answer.
pub fn trust_prompt_installed() -> bool {
    PROMPT.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// What came back from the question. "Nobody was asked" is its own
/// answer and not a no: the two have different causes, so they have
/// different sentences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Yes,
    No,
    /// No question is installed on this host.
    Unasked,
}

fn ask_to_trust(request: &TrustRequest) -> Answer {
    let ask = PROMPT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .cloned();
    // The person's own answer, in their own time: joy's contact bound
    // is held open while the question stands (`super::bound`).
    let _hold = ask.is_some().then(super::bound::hold);
    match ask {
        Some(ask) if ask(request) => Answer::Yes,
        Some(_) => Answer::No,
        None => Answer::Unasked,
    }
}

thread_local! {
    /// joy's own refusal sentence for the contact running on THIS
    /// thread. libgit2 calls the callback synchronously on the
    /// contact's own thread (the same reason
    /// `contact::note_credential_presented` is a thread local), so this
    /// is exactly the scope of one contact.
    static REFUSAL: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// What the x509 branch read off the certificate, for the detail
    /// line of `tls_untrusted` (D1.8c, package J4p).
    static X509: std::cell::RefCell<Option<X509Note>> = const { std::cell::RefCell::new(None) };
}

/// The issuer and the subject of the certificate an https contact was
/// offered, for the detail line. Neither is a verdict: the branch that
/// fills this returns `CertificatePassthrough` and leaves the decision
/// to libgit2.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct X509Note {
    /// The issuer's common name, when the certificate has one.
    pub issuer: Option<String>,
    /// The subject's common name, when the certificate has one.
    pub subject: Option<String>,
}

/// Read joy's own refusal sentence for this contact and clear it.
///
/// The classifier calls this for a certificate code on an ssh contact:
/// the state is `needs_host_trust` either way, and this is what turns
/// libgit2's "invalid or unknown remote ssh hostkey" into the sentence
/// that names the host, the fingerprint and the next step.
pub fn take_refusal() -> Option<String> {
    REFUSAL.with(|cell| cell.borrow_mut().take())
}

/// Forget whatever the last contact on this thread left behind. Called
/// by the contact boundary before the work runs.
pub fn forget_refusal() {
    REFUSAL.with(|cell| *cell.borrow_mut() = None);
    X509.with(|cell| *cell.borrow_mut() = None);
}

/// What the x509 branch saw on this thread's last contact.
pub fn x509_note() -> Option<X509Note> {
    X509.with(|cell| cell.borrow().clone())
}

fn note_refusal(sentence: &str) {
    REFUSAL.with(|cell| *cell.borrow_mut() = Some(sentence.to_string()));
}

/// The closure that goes into every `RemoteCallbacks` joy builds.
///
/// `configured` is the remote URL as the person wrote it, which is what
/// carries the port and the `Host` alias: libgit2 hands the callback
/// the host it dialled and nothing else.
pub(crate) fn check(
    kind: HostKind,
    configured: Option<String>,
) -> impl FnMut(&Cert<'_>, &str) -> Result<Status, git2::Error> + 'static {
    let mut trust = Trust::new(kind, configured);
    move |cert, host| trust.certificate(cert, host)
}

/// The state of one contact's certificate check.
struct Trust {
    kind: HostKind,
    configured: Option<String>,
    /// The host libgit2 dialled, with the ssh config settings and the
    /// port joy resolved for it. Resolved once per host and kept, so a
    /// contact that re-enters the callback reads no config twice.
    site: Option<(String, HostSettings, u16)>,
}

impl Trust {
    fn new(kind: HostKind, configured: Option<String>) -> Trust {
        Trust {
            kind,
            configured,
            site: None,
        }
    }

    fn certificate(&mut self, cert: &Cert<'_>, host: &str) -> Result<Status, git2::Error> {
        if let Some(hostkey) = cert.as_hostkey() {
            return self.host_key(host, hostkey.hostkey_type(), hostkey.hostkey());
        }
        if let Some(x509) = cert.as_x509() {
            return self.x509(x509.data());
        }
        // A kind joy does not know is libgit2's business, not joy's.
        Ok(Status::CertificatePassthrough)
    }

    /// The x509 branch: read something for the detail line, decide
    /// nothing (D1.8c).
    fn x509(&mut self, der: &[u8]) -> Result<Status, git2::Error> {
        let (issuer, subject) = der_names(der);
        let note = X509Note { issuer, subject };
        tracing::debug!(
            issuer = note.issuer.as_deref().unwrap_or("unknown"),
            subject = note.subject.as_deref().unwrap_or("unknown"),
            "https certificate seen; libgit2 decides"
        );
        X509.with(|cell| *cell.borrow_mut() = Some(note));
        Ok(Status::CertificatePassthrough)
    }

    /// The ssh config and the port for the host libgit2 dialled.
    fn site(&mut self, host: &str) -> (&HostSettings, u16) {
        let stale = match &self.site {
            Some((known, _, _)) => !known.eq_ignore_ascii_case(host),
            None => true,
        };
        if stale {
            let settings = super::ssh_config::for_contact(host, self.configured.as_deref());
            let port = self.port_for(host, &settings);
            self.site = Some((host.to_string(), settings, port));
        }
        let (_, settings, port) = self.site.as_ref().expect("the site was just resolved");
        (settings, *port)
    }

    /// The port this contact is really on. libgit2 passes none, so it
    /// comes from the remote URL the person configured (which beats the
    /// config, as ssh's command line does), then from `Port`, then from
    /// ssh's own default.
    fn port_for(&self, host: &str, settings: &HostSettings) -> u16 {
        let from_url = self.configured.as_deref().and_then(|url| {
            let parsed = super::remote_url::RemoteUrl::parse(url)?;
            if parsed.transport != super::remote_url::Transport::Ssh {
                return None;
            }
            // The URL speaks about this contact only when it names this
            // host, directly or through the alias the config renamed.
            let names_it = parsed.host.eq_ignore_ascii_case(host)
                || super::ssh_config::for_host(&parsed.host)
                    .effective_host(&parsed.host)
                    .eq_ignore_ascii_case(host);
            names_it.then_some(parsed.port).flatten()
        });
        from_url
            .or(settings.port)
            .unwrap_or(known_hosts::DEFAULT_PORT)
    }

    /// The whole check of D1.4a, in Rust, answering `CertificateOk` or
    /// an error and never passthrough.
    fn host_key(
        &mut self,
        host: &str,
        key_type: Option<SshHostKeyType>,
        key: Option<&[u8]>,
    ) -> Result<Status, git2::Error> {
        let Some(key) = key else {
            // libgit2 refuses this itself ("unable to get the host
            // key", ssh_libssh2.c:748), so this is the second line of
            // defence and never the normal path.
            return refuse(format!(
                "joy could not read the host key {host} presented, so it cannot check it."
            ));
        };
        let named = known_hosts::key_type_in_blob(key);
        if let Some(name) = named.as_deref() {
            if known_hosts::is_certificate_type(name) {
                return refuse(format!(
                    "joy cannot verify certificate host keys yet, this host presents one \
                     ({host}, {name})."
                ));
            }
        }
        // The blob names its own type, which is the known_hosts
        // spelling; git2's name is the fallback, and it reads "unknown"
        // for every type it has no variant for (cert.rs:52-62).
        let key_type = match named {
            Some(name) => name,
            None => match key_type {
                Some(SshHostKeyType::Unknown) | None => {
                    return refuse(format!(
                        "joy does not recognise the host key type {host} presented."
                    ))
                }
                Some(known) => known.name().to_string(),
            },
        };
        let (settings, port) = self.site(host);
        let files: Vec<PathBuf> = settings
            .user_known_hosts
            .iter()
            .chain(settings.global_known_hosts.iter())
            .cloned()
            .collect();
        let verdict = known_hosts::look_up(&files, host, port, &key_type, key);
        tracing::debug!(host, port, key_type, verdict = ?verdict, "host key checked");
        match verdict {
            Verdict::Known { .. } => Ok(Status::CertificateOk),
            Verdict::Revoked { file, line } => refuse(format!(
                "The host key {host} (port {port}) presents is marked @revoked in {} line {line}. \
                 A revoked key is never accepted for authentication.",
                file.display()
            )),
            Verdict::Mismatch {
                file,
                line,
                expected,
            } => refuse(format!(
                "The host key of {host} (port {port}) changed. It presents {key_type} {}, \
                 and {} line {line} holds {expected}. joy never accepts a changed key: ask the \
                 forge whether the key really changed, then remove line {line} by hand.",
                known_hosts::fingerprint(key),
                file.display()
            )),
            Verdict::UnknownKeyType { known_types } => {
                self.unseen(host, port, &key_type, key, known_types, None)
            }
            Verdict::Unknown => match self.pin_says(host, &key_type, key) {
                Pin::Mismatch(sentence) => refuse(sentence),
                Pin::Published(note) => self.unseen(host, port, &key_type, key, Vec::new(), note),
            },
        }
    }

    /// What the pinned keys of the three public forges say about this
    /// key. Consulted only where no file holds a line for the host
    /// (D1.4a), and only for the hosts the pin file names.
    fn pin_says(&self, host: &str, key_type: &str, key: &[u8]) -> Pin {
        let Some(pin) = pins::consulted_for(host) else {
            return Pin::Published(None);
        };
        let Some(pinned) = pin
            .keys
            .iter()
            .find(|k| k.key_type.eq_ignore_ascii_case(key_type))
        else {
            // The forge publishes no key of this type, so the pin says
            // nothing about it and the ordinary rule applies.
            return Pin::Published(None);
        };
        if pinned.blob().as_deref() == Some(key) {
            return Pin::Published(Some(format!(
                "this is the key {} publishes at {}",
                pin.forge, pin.published_at
            )));
        }
        Pin::Mismatch(format!(
            "The host key of {host} is not the one {} publishes. It presents {key_type} {}, \
             and joy {} pins {}. Either this connection is not going to {}, or the key was \
             rotated and this joy is too old: check {} and update joy.",
            pin.forge,
            known_hosts::fingerprint(key),
            env!("CARGO_PKG_VERSION"),
            pinned.fingerprint,
            pin.forge,
            pin.published_at
        ))
    }

    /// A host key no file of this machine speaks about: the rule per
    /// host kind (D1.4a).
    fn unseen(
        &mut self,
        host: &str,
        port: u16,
        key_type: &str,
        key: &[u8],
        other_types: Vec<String>,
        published: Option<String>,
    ) -> Result<Status, git2::Error> {
        let kind = self.kind;
        let (settings, _) = self.site(host);
        let strict = settings.strict_host_keys;
        let hashed = settings.hash_known_hosts;
        let Some(file) = settings.user_known_hosts.first().cloned() else {
            return refuse(format!(
                "This machine has never seen the host key of {host} (port {port}), and your ssh \
                 config sets UserKnownHostsFile to none, so joy has no file to record it in."
            ));
        };
        let fingerprint = known_hosts::fingerprint(key);
        // What joy would WRITE follows the person's config; what a
        // person is asked to PASTE is the plain form, because that is
        // the line they can read and compare with what the forge
        // publishes.
        let to_write = known_hosts::line_for(host, port, key_type, key, hashed, None);
        let to_paste = known_hosts::line_for(host, port, key_type, key, false, None);
        let mut known_for = String::new();
        if !other_types.is_empty() {
            known_for = format!(
                " This machine knows {host} for {}, and not for {key_type}.",
                other_types.join(", ")
            );
        }
        // Every refusal on this path ends in the same next step
        // (D1.8b: one, and named): the line to paste and the file it
        // goes in. Only the middle clause differs, because only the
        // reason differs.
        let unseen_refusal = |because: &str| {
            format!(
                "This machine has never seen the host key of {host} (port {port}): {key_type} \
                 {fingerprint}.{known_for} {because} To trust it, add this line to {}: {}",
                file.display(),
                to_paste.trim_end()
            )
        };
        // The person's own `StrictHostKeyChecking yes` refuses before
        // anything else may accept, pins included: it is the one
        // setting that says "never add a host for me".
        if strict == StrictHostKeys::Yes {
            return refuse(format!(
                "This machine has never seen the host key of {host} (port {port}): {key_type} \
                 {fingerprint}.{known_for} Your ssh config sets StrictHostKeyChecking yes for \
                 this host, so joy neither asks nor adds it."
            ));
        }
        if !kind.may_prompt() {
            // `Background` and `Delegated`: never ask, never write, in
            // every StrictHostKeyChecking mode (D1.4a). The one key
            // they accept without a file is the pinned one, and they
            // record nothing about it either, so a pin that is replaced
            // takes effect at once.
            if let Some(published) = &published {
                tracing::info!(host, port, key_type, published, "pinned host key accepted");
                return Ok(Status::CertificateOk);
            }
            return refuse(unseen_refusal("Nobody can be asked here, so joy refuses."));
        }
        let answer = match strict {
            // `accept-new`, and `no` and `off` with it: add it without
            // asking. joy never skips the check itself, it only skips
            // the question.
            StrictHostKeys::AcceptNew => Answer::Yes,
            StrictHostKeys::Ask => ask_to_trust(&TrustRequest {
                host: host.to_string(),
                port,
                key_type: key_type.to_string(),
                fingerprint: fingerprint.clone(),
                file: file.clone(),
                line: to_write.clone(),
                published,
                other_types,
            }),
            // returned above, before anything could accept
            StrictHostKeys::Yes => Answer::No,
        };
        match answer {
            Answer::Yes => {}
            Answer::No => {
                return refuse(format!(
                    "The host key of {host} (port {port}) was not trusted: {key_type} \
                     {fingerprint}. joy added nothing. If you decide to trust it after all, add \
                     this line to {}: {}",
                    file.display(),
                    to_paste.trim_end()
                ))
            }
            // An `Interactive` host whose front end lends joy-core no
            // terminal: the host kind says a person is there, and
            // nothing here can reach them, so this refusal says both
            // and still names the next step.
            Answer::Unasked => {
                return refuse(unseen_refusal(
                    "This joy has no way to put the question to you, so it refuses.",
                ))
            }
        }
        if let Err(e) = known_hosts::append(&file, &to_write) {
            return refuse(format!(
                "joy could not add the host key of {host} to {}: {e}",
                file.display()
            ));
        }
        tracing::info!(host, port, key_type, "host key trusted and recorded");
        Ok(Status::CertificateOk)
    }
}

#[cfg(test)]
impl Trust {
    /// A check whose site is already resolved, so a test decides
    /// against the files it wrote and not against the machine's own
    /// ssh config.
    fn at(kind: HostKind, host: &str, settings: HostSettings, port: u16) -> Trust {
        Trust {
            kind,
            configured: None,
            site: Some((host.to_string(), settings, port)),
        }
    }

    /// The same closure [`check`] installs, around a site a test chose.
    /// Lets a real libgit2 ssh contact run against known_hosts files a
    /// test wrote instead of the machine's own. Only the network tests
    /// need it, so it is gated on their feature: a plain `cargo test -p
    /// joy-core` then has no dead code to warn about.
    #[cfg(feature = "forge-net")]
    fn into_check(
        mut self,
    ) -> impl FnMut(&Cert<'_>, &str) -> Result<Status, git2::Error> + 'static {
        move |cert, host| self.certificate(cert, host)
    }
}

/// What the pin file says about a key no known_hosts file knows.
enum Pin {
    /// Nothing, or "this is the published key", which is a note for the
    /// question and not a verdict of its own.
    Published(Option<String>),
    /// The host is pinned and presented another key.
    Mismatch(String),
}

/// Refuse this contact with joy's own sentence.
///
/// The CODE is what survives libgit2's overwrite and what the
/// classifier reads (`GIT_ECERTIFICATE` on an ssh contact is
/// `needs_host_trust`, D1.8b); the sentence travels in the cell and is
/// substituted when the operation returns.
fn refuse(sentence: String) -> Result<Status, git2::Error> {
    tracing::debug!(sentence, "host key refused");
    note_refusal(&sentence);
    Err(git2::Error::new(
        git2::ErrorCode::Certificate,
        git2::ErrorClass::Ssh,
        sentence,
    ))
}

// ---- just enough DER to name a certificate ---------------------------

/// The issuer's and the subject's common name of a DER certificate.
///
/// This is not a certificate parser and must never grow into one: the
/// x509 branch decides nothing, so the worst a misread here can do is
/// leave a detail line without a name. `Certificate` is a SEQUENCE
/// whose first element is `TBSCertificate`, itself a SEQUENCE of an
/// optional `[0] version`, a serial `INTEGER`, a signature `SEQUENCE`,
/// the issuer `Name`, a validity `SEQUENCE` and the subject `Name`
/// (RFC 5280 4.1), so both names are reached by counting elements.
fn der_names(der: &[u8]) -> (Option<String>, Option<String>) {
    let names = || -> Option<(Option<String>, Option<String>)> {
        // Certificate ::= SEQUENCE { tbsCertificate, ... }
        let certificate = der_element(der, 0)?.1;
        let tbs = der_element(certificate, 0)?.1;
        let mut at = 0usize;
        let mut element = der_element(tbs, at)?;
        // the optional explicit [0] version
        if element.0 == 0xa0 {
            at += element.2;
            element = der_element(tbs, at)?;
        }
        // serialNumber
        at += element.2;
        // signature AlgorithmIdentifier
        at += der_element(tbs, at)?.2;
        let issuer = der_element(tbs, at)?;
        at += issuer.2;
        // validity
        at += der_element(tbs, at)?.2;
        let subject = der_element(tbs, at)?;
        Some((common_name(issuer.1), common_name(subject.1)))
    };
    names().unwrap_or((None, None))
}

/// One DER element of `body` at `at`: its tag, its contents, and how
/// many bytes the whole element took.
fn der_element(body: &[u8], at: usize) -> Option<(u8, &[u8], usize)> {
    let tag = *body.get(at)?;
    let first = *body.get(at + 1)? as usize;
    let (length, header) = if first < 0x80 {
        (first, 2)
    } else {
        let count = first & 0x7f;
        // a length of more than four bytes is not a certificate joy
        // will meet, and reading one would be the only way to overflow
        if count == 0 || count > 4 {
            return None;
        }
        let mut length = 0usize;
        for offset in 0..count {
            length = (length << 8) | *body.get(at + 2 + offset)? as usize;
        }
        (length, 2 + count)
    };
    let contents = body.get(at + header..at + header + length)?;
    Some((tag, contents, header + length))
}

/// The `CN` of a `Name`, which is a SEQUENCE of RDN SETs. The
/// attribute's OID is `2.5.4.3`, `06 03 55 04 03` in DER.
fn common_name(name: &[u8]) -> Option<String> {
    const CN_OID: [u8; 5] = [0x06, 0x03, 0x55, 0x04, 0x03];
    let at = name
        .windows(CN_OID.len())
        .position(|window| window == CN_OID)?;
    let (_, value, _) = der_element(name, at + CN_OID.len())?;
    std::str::from_utf8(value).ok().map(str::to_string)
}

#[cfg(test)]
mod tests;
