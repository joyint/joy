// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The known_hosts files, read by joy itself (design D1.4a).
//!
//! libgit2 reads ONE file and matches it with `strcmp`: `~/.ssh/known_hosts`
//! (`SSH_DIR ".ssh"`, `KNOWN_HOSTS_FILE "known_hosts"`,
//! ssh_libssh2.c:426-427) through `libssh2_knownhost_readfile`, whose
//! matcher compares plain names byte for byte (knownhost.c:407-414).
//! So `known_hosts2`, `/etc/ssh/ssh_known_hosts`, every
//! `UserKnownHostsFile` and `GlobalKnownHostsFile` the person wrote,
//! every wildcard pattern, `@revoked` and `@cert-authority` are
//! invisible to a joy contact unless joy reads them, which is what this
//! module does. The hashed form IS read by libssh2, but a person on
//! Ubuntu has hashed entries for every host (`HashKnownHosts yes` in
//! /etc/ssh/ssh_config), so joy has to read it as well to answer "do I
//! know this host" without libgit2.
//!
//! Two things here are not obvious and both are load bearing:
//!
//! - **One unparsable line kills the whole file.**
//!   `libssh2_knownhost_readfile` stops at the first line its parser
//!   refuses and returns a negative value for the file
//!   ("Failed to parse known hosts file", knownhost.c:960-985), which
//!   libgit2 turns into "error reading known_hosts"
//!   (ssh_libssh2.c:443, :466) and which ends the connection. So joy
//!   validates `~/.ssh/known_hosts` with libssh2's OWN rules
//!   ([`validate_text`]) before the first ssh contact of a process and
//!   names the line number, instead of letting a person read
//!   "error reading known_hosts" about a file with four hundred lines.
//! - **A hashed line is the normal case, not the exception.** Ubuntu
//!   ships `HashKnownHosts yes`, so the file holds
//!   `|1|<salt>|<hash>` and the name is recoverable only by computing
//!   `HMAC-SHA1(salt, name)` per line (knownhost.c:416-441).

use std::io::Write;
use std::path::{Path, PathBuf};

use base64ct::Encoding;

/// The port ssh contacts unless something says otherwise. A line for
/// any other port carries the bracketed form.
pub const DEFAULT_PORT: u16 = 22;

/// The marker a line may carry in front of the host field (man sshd).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    /// No marker: an ordinary line.
    None,
    /// `@revoked`: a key that is "never accepted for authentication",
    /// whatever the host kind and whatever else the files say.
    Revoked,
    /// `@cert-authority`: a CA that signs host certificates. Out of
    /// scope for this version (D1.4a), and therefore never a match.
    CertAuthority,
}

/// The host field of one line: a pattern list, or the hashed form.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HostField {
    /// `github.com`, `*.example.com,!build.example.com`,
    /// `[git.example.com]:2222`.
    Patterns(Vec<String>),
    /// `|1|<base64 salt>|<base64 hash>`, matched by computing
    /// `HMAC-SHA1(salt, name)`.
    Hashed { salt: Vec<u8>, hash: Vec<u8> },
}

/// One line of a known_hosts file, as joy reads it.
#[derive(Debug, Clone)]
pub struct Entry {
    marker: Marker,
    host: HostField,
    /// The key type as the file spells it (`ssh-ed25519`).
    key_type: String,
    /// The raw key blob, decoded from the line's base64 field.
    key: Vec<u8>,
    /// The one based line number, for a sentence that names it.
    line: usize,
}

impl Entry {
    /// The key type this line carries.
    pub fn key_type(&self) -> &str {
        &self.key_type
    }

    /// The line number in its file, one based, as a person counts.
    pub fn line(&self) -> usize {
        self.line
    }

    /// The fingerprint of the key this line stores, in the spelling the
    /// forges publish.
    pub fn fingerprint(&self) -> String {
        fingerprint(&self.key)
    }

    /// Whether this line speaks about `name`, which is the host as ssh
    /// writes it: `github.com`, or `[git.example.com]:2222` for a port
    /// other than 22.
    fn matches_host(&self, name: &str) -> bool {
        match &self.host {
            HostField::Patterns(patterns) => {
                let mut positive = false;
                for pattern in patterns {
                    match pattern.strip_prefix('!') {
                        // one negated pattern refuses the whole line,
                        // however many positive ones match (man sshd)
                        Some(negated) => {
                            if wildcard_match(negated, name) {
                                return false;
                            }
                        }
                        None => positive |= wildcard_match(pattern, name),
                    }
                }
                positive
            }
            HostField::Hashed { salt, hash } => hmac_sha1(salt, name.as_bytes()) == hash.as_slice(),
        }
    }
}

/// What the files say about the key a host just presented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A line names this host and this exact key: the host is known.
    Known { file: PathBuf, line: usize },
    /// A line names this host and this exact key and carries
    /// `@revoked`. Refused in every host kind.
    Revoked { file: PathBuf, line: usize },
    /// A line names this host and this key TYPE with another key. The
    /// one case joy never writes and never accepts.
    Mismatch {
        file: PathBuf,
        line: usize,
        /// The fingerprint the file expects.
        expected: String,
    },
    /// Lines name this host, but none for this key type. Not a
    /// mismatch: ssh would offer to add this type, and the sentence has
    /// to say which types are already there.
    UnknownKeyType { known_types: Vec<String> },
    /// No file holds any line for this host. The only state in which a
    /// pin may be consulted (D1.4a).
    Unknown,
}

/// What every file joy reads says about one presented host key.
///
/// The files are read in the order ssh reads them (every
/// `UserKnownHostsFile` first, then every `GlobalKnownHostsFile`), and
/// the whole set is read before the verdict is formed, because a
/// `@revoked` line in the last file beats a match in the first one.
pub fn look_up(files: &[PathBuf], host: &str, port: u16, key_type: &str, key: &[u8]) -> Verdict {
    let name = host_field(host, port);
    let mut mismatch: Option<Verdict> = None;
    let mut known_types: Vec<String> = Vec::new();
    let mut matched: Option<Verdict> = None;
    for file in files {
        let Some(text) = read_text(file) else {
            continue;
        };
        for entry in parse(&text) {
            if entry.marker == Marker::CertAuthority || !entry.matches_host(&name) {
                continue;
            }
            if entry.marker == Marker::Revoked {
                // unconditional, and it beats every other line, in
                // every file: man sshd says a revoked key is "never
                // accepted for authentication". A revoked line about
                // ANOTHER key says nothing about this one, so it is
                // neither a mismatch nor a key type this host has.
                if entry.key == key {
                    return Verdict::Revoked {
                        file: file.clone(),
                        line: entry.line,
                    };
                }
                continue;
            }
            if !entry.key_type.eq_ignore_ascii_case(key_type) {
                if !known_types.iter().any(|t| t == &entry.key_type) {
                    known_types.push(entry.key_type.clone());
                }
                continue;
            }
            if entry.key == key {
                matched.get_or_insert(Verdict::Known {
                    file: file.clone(),
                    line: entry.line,
                });
            } else if mismatch.is_none() {
                mismatch = Some(Verdict::Mismatch {
                    file: file.clone(),
                    line: entry.line,
                    expected: entry.fingerprint(),
                });
            }
        }
    }
    // A key that a line holds is known even when another line of the
    // same type disagrees: that is a host with two keys of one type,
    // which ssh accepts as well.
    if let Some(known) = matched {
        return known;
    }
    if let Some(mismatch) = mismatch {
        return mismatch;
    }
    if !known_types.is_empty() {
        return Verdict::UnknownKeyType { known_types };
    }
    Verdict::Unknown
}

/// Every line of a known_hosts text joy could read. A line joy cannot
/// read is skipped here and reported by [`validate_text`], which is the
/// one place that speaks about a broken file.
pub fn parse(text: &str) -> Vec<Entry> {
    text.lines()
        .enumerate()
        .filter_map(|(index, line)| parse_line(line, index + 1))
        .collect()
}

fn parse_line(raw: &str, line: usize) -> Option<Entry> {
    let text = raw.trim_start_matches([' ', '\t']);
    if text.is_empty() || text.starts_with('#') {
        return None;
    }
    let mut fields = text.split([' ', '\t']).filter(|f| !f.is_empty());
    let mut first = fields.next()?;
    let marker = match first {
        "@revoked" => {
            first = fields.next()?;
            Marker::Revoked
        }
        "@cert-authority" => {
            first = fields.next()?;
            Marker::CertAuthority
        }
        _ => Marker::None,
    };
    let key_type = fields.next()?.to_string();
    let key = decode_base64(fields.next()?)?;
    let host = match first.strip_prefix("|1|") {
        Some(rest) => {
            let (salt, hash) = rest.split_once('|')?;
            HostField::Hashed {
                salt: decode_base64(salt)?,
                hash: decode_base64(hash)?,
            }
        }
        None => HostField::Patterns(first.split(',').map(str::to_string).collect()),
    };
    Some(Entry {
        marker,
        host,
        key_type,
        key,
        line,
    })
}

/// The host as ssh writes it in a file and hashes it for a hashed
/// entry: the bare name on port 22, the bracketed form on every other
/// port.
pub fn host_field(host: &str, port: u16) -> String {
    let host = host.to_ascii_lowercase();
    if port == DEFAULT_PORT {
        host
    } else {
        format!("[{host}]:{port}")
    }
}

/// The fingerprint string the forges publish and ssh prints:
/// `SHA256:` and the unpadded base64 of the SHA-256 of the raw blob.
pub fn fingerprint(key: &[u8]) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(key);
    format!(
        "SHA256:{}",
        base64ct::Base64Unpadded::encode_string(&digest)
    )
}

/// The key type a raw host key blob names itself, which is the first
/// ssh string field of the blob (`ssh-ed25519`,
/// `ssh-ed25519-cert-v01@openssh.com`). joy reads it because git2
/// reports every type it does not know as `SshHostKeyType::Unknown`
/// with no name at all (cert.rs:52-62).
pub fn key_type_in_blob(key: &[u8]) -> Option<String> {
    let (length, rest) = key.split_at_checked(4)?;
    let length = u32::from_be_bytes([length[0], length[1], length[2], length[3]]) as usize;
    // a name longer than this is not a key type, it is a broken blob
    if length == 0 || length > 64 || length > rest.len() {
        return None;
    }
    std::str::from_utf8(&rest[..length])
        .ok()
        .map(str::to_string)
}

/// Whether a key type is an OpenSSH host CERTIFICATE rather than a key.
/// joy refuses those by name: `@cert-authority` is out of scope for
/// this version, and a certificate arrives with no usable type from
/// git2 at all (D1.4a).
pub fn is_certificate_type(key_type: &str) -> bool {
    key_type.ends_with("-cert-v01@openssh.com")
}

/// The line joy would write for this key, exactly as it writes it:
/// hashed when the person's config says `HashKnownHosts yes`, bracketed
/// when the port is not 22.
///
/// `salt` is the hash salt, so a test can write the same line twice;
/// production passes `None` and gets twenty random bytes, which is the
/// digest length OpenSSH uses.
pub fn line_for(
    host: &str,
    port: u16,
    key_type: &str,
    key: &[u8],
    hashed: bool,
    salt: Option<&[u8]>,
) -> String {
    let name = host_field(host, port);
    let field = if hashed {
        let salt = match salt {
            Some(salt) => salt.to_vec(),
            None => {
                use rand::RngCore;
                let mut salt = vec![0u8; 20];
                rand::thread_rng().fill_bytes(&mut salt);
                salt
            }
        };
        let hash = hmac_sha1(&salt, name.as_bytes());
        format!(
            "|1|{}|{}",
            base64ct::Base64::encode_string(&salt),
            base64ct::Base64::encode_string(&hash)
        )
    } else {
        name
    };
    format!(
        "{field} {key_type} {}\n",
        base64ct::Base64::encode_string(key)
    )
}

/// Append one line to the first `UserKnownHostsFile`, under the cross
/// process lock, and answer the line that was written.
///
/// The directory is created 0700 and the file 0600 when they are
/// missing, which is what ssh requires of them. An existing line is
/// never rewritten and the file is never truncated: joy only ever adds.
///
/// The lock is [`crate::util::file_lock`] (design D2.6a, landed by
/// J4a), held on a lock file of joy's own under the state directory
/// rather than on `known_hosts` itself: on Windows the lock is
/// MANDATORY for the locked range, so locking the file a concurrent
/// contact is reading would turn a second joy thread's contact into
/// "error reading known_hosts".
pub fn append(file: &Path, line: &str) -> Result<(), String> {
    if let Some(parent) = file.parent() {
        create_ssh_dir(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let _lock = lock_for(file);
    let mut handle = open_private(file).map_err(|e| format!("{}: {e}", file.display()))?;
    handle
        .write_all(line.as_bytes())
        .and_then(|()| handle.flush())
        .map_err(|e| format!("{}: {e}", file.display()))
}

/// The lock that guards one known_hosts append. A lock joy cannot take
/// is not a reason to refuse the contact: the append is one `write` of
/// one line opened for append, which no operating system tears, and the
/// lock is there so two joy processes do not interleave a
/// read-modify-write, not to make the write itself atomic.
fn lock_for(file: &Path) -> Option<crate::util::file_lock::FileLock> {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(file.to_string_lossy().as_bytes());
    let name = format!("known-hosts-{}.lock", hex::encode(&digest[..8]));
    let path = crate::auth::session::app_state_dir()
        .ok()?
        .join("locks")
        .join(name);
    match crate::util::file_lock::exclusive(&path, crate::util::file_lock::DEFAULT_WAIT) {
        Ok(lock) => Some(lock),
        Err(e) => {
            tracing::debug!(error = %e, "known_hosts append is not locked");
            None
        }
    }
}

#[cfg(unix)]
fn create_ssh_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    if dir.exists() {
        return Ok(());
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_ssh_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

#[cfg(unix)]
fn open_private(file: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(file)
}

#[cfg(not(unix))]
fn open_private(file: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(file)
}

// ---- the pre validation of ~/.ssh/known_hosts ------------------------

/// A line libssh2 refuses, which means the whole file is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    /// One based, as a person counts lines in an editor.
    pub line: usize,
    /// What is wrong with it, in joy's words.
    pub reason: &'static str,
}

/// The first line of `text` that `libssh2_knownhost_readfile` would
/// refuse, and with it the whole file.
///
/// These are libssh2's own rules, read off knownhost.c, and not a
/// stricter reading of man sshd: the point is to name the line libssh2
/// will die on, so every rule here has its source beside it.
pub fn validate_text(text: &str) -> Option<Fault> {
    validate_bytes(text.as_bytes())
}

/// The same rules over the BYTES a file really holds.
///
/// libssh2 and OpenSSH read known_hosts as bytes, and joy reads it the
/// same way: a comment field copied out of a key file carries whatever
/// bytes that file carried, and a line that is not UTF-8 is a line
/// libssh2 still parses. Counting bytes is also the only way to index
/// a line safely: `text[3..]` on a host field of two characters and a
/// key field that starts with a multi-byte one would panic on a char
/// boundary, inside the check that is there to keep one bad line from
/// killing every ssh contact.
pub fn validate_bytes(bytes: &[u8]) -> Option<Fault> {
    bytes
        .split(|byte| *byte == b'\n')
        .enumerate()
        .find_map(|(index, line)| {
            // a file written on Windows is a file, and `str::lines`
            // drops the carriage return as well
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            line_fault(line).map(|reason| Fault {
                line: index + 1,
                reason,
            })
        })
}

fn line_fault(raw: &[u8]) -> Option<&'static str> {
    // libssh2_knownhost_readline, knownhost.c:879-945
    let text = trim_blanks(raw);
    if text.first() == Some(&b'#') {
        return None;
    }
    let Some(at) = text.iter().position(|byte| *byte == b' ' || *byte == b'\t') else {
        // "illegal line": a host and no key at all, and an empty line,
        // which libssh2 skips
        return (!text.is_empty()).then_some("the line names a host and no key");
    };
    let host = &text[..at];
    let rest = trim_blanks(&text[at..]);
    if rest.is_empty() {
        return Some("the line names a host and no key");
    }
    // hostline, knownhost.c:751-757: the key field including its
    // comment has to be at least twenty characters
    if rest.len() < 20 {
        return Some("the key field is shorter than twenty characters");
    }
    if host.len() > 2 && !host.starts_with(b"|1|") {
        // oldstyle_hostline, knownhost.c:620-647
        if host
            .split(|byte| *byte == b',')
            .any(|name| name.len() >= 255)
        {
            return Some("a host name on the line is longer than 254 characters");
        }
        return None;
    }
    // hashed_hostline, knownhost.c:674-728. libssh2 takes this branch
    // for a host field SHORTER than three characters as well, and then
    // reads the salt out of the key field, which is why the text below
    // is the whole line from the host field's fourth byte and not the
    // host field alone.
    let after_marker = &text[3.min(text.len())..];
    let Some(bar) = after_marker.iter().position(|byte| *byte == b'|') else {
        // no separator: libssh2 returns 0 and simply stores nothing
        return None;
    };
    let salt = &after_marker[..bar];
    if salt.len() >= 31 {
        return Some("the salt of the hashed host field is too long");
    }
    // `hostlen -= 3` and then `hostlen -= saltlen + 1` on a size_t: a
    // host field that cannot carry both underflows and trips libssh2's
    // own length check
    let Some(hash_len) = host
        .len()
        .checked_sub(3)
        .and_then(|len| len.checked_sub(salt.len() + 1))
    else {
        return Some("the hashed host field is malformed");
    };
    if hash_len >= 255 {
        return Some("the hash of the hashed host field is too long");
    }
    let hash = &after_marker[bar + 1..][..hash_len.min(after_marker.len() - bar - 1)];
    // _libssh2_base64_decode skips every character outside the
    // alphabet, including '=', and fails on exactly one thing: a count
    // of base64 characters that leaves a lone partial octet
    // (misc.c:396-424)
    if base64_digits(salt) % 4 == 1 || base64_digits(hash) % 4 == 1 {
        return Some("the hashed host field is not base64");
    }
    None
}

/// The refusal sentence for `~/.ssh/known_hosts` when libssh2 would
/// choke on it, `None` when the file is fine or absent.
///
/// The file is read ONCE per process: it is the file libgit2 reads
/// itself (`SSH_DIR ".ssh"`, `KNOWN_HOSTS_FILE "known_hosts"`,
/// ssh_libssh2.c:426-427) and it is read before every ssh contact, so
/// checking it per contact would be one stat and one parse per fetch
/// for a fault that does not appear while a process runs.
pub fn user_file_refusal() -> Option<String> {
    static CHECKED: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    CHECKED
        .get_or_init(|| {
            let path = user_file()?;
            refusal_for(&path, &read_bytes(&path)?)
        })
        .clone()
}

/// The sentence for one file and the bytes it holds.
fn refusal_for(path: &Path, text: &[u8]) -> Option<String> {
    let fault = validate_bytes(text)?;
    Some(format!(
        "{} line {}: {}. libssh2 reads this file before joy does and refuses the WHOLE file for \
         one line it cannot parse, so every ssh contact fails until that line is repaired or \
         removed.",
        path.display(),
        fault.line,
        fault.reason
    ))
}

/// The one file libgit2 reads by itself, `~/.ssh/known_hosts`.
pub fn user_file() -> Option<PathBuf> {
    Some(super::ssh_config::user_config_path()?.with_file_name("known_hosts"))
}

fn base64_digits(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .filter(|b| b.is_ascii_alphanumeric() || **b == b'+' || **b == b'/')
        .count()
}

/// A line without its leading blanks.
fn trim_blanks(bytes: &[u8]) -> &[u8] {
    let at = bytes
        .iter()
        .position(|byte| *byte != b' ' && *byte != b'\t')
        .unwrap_or(bytes.len());
    &bytes[at..]
}

/// The bytes of one known_hosts file, with every skip said out loud.
///
/// A file that is not there is the normal case and the quiet one. A
/// file joy may not read is not: ssh reads it, so joy would decide
/// about a host on less than ssh knows, and a `Background` host would
/// refuse a host the file names.
fn read_bytes(file: &Path) -> Option<Vec<u8>> {
    match std::fs::read(file) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(file = %file.display(), "no known_hosts file here");
            None
        }
        Err(e) => {
            tracing::warn!(
                file = %file.display(),
                error = %e,
                "a known_hosts file was skipped, so joy knows less about this host than ssh does"
            );
            None
        }
    }
}

/// The text of one known_hosts file, decoded the way it has to be
/// decoded: every field joy reads is ASCII (a host name, a key type, a
/// base64 blob), so a byte no UTF-8 decoder accepts can only sit in a
/// comment, and reading it as a replacement character changes no host,
/// no type and no key. Dropping the whole file for such a byte would
/// make joy call a host unknown that the file names, which a
/// `Background` host then refuses and an `accept-new` host answers
/// with a duplicate line.
fn read_text(file: &Path) -> Option<String> {
    match String::from_utf8(read_bytes(file)?) {
        Ok(text) => Some(text),
        Err(e) => {
            tracing::debug!(
                file = %file.display(),
                "a known_hosts file holds bytes that are not UTF-8; they are read as replacement \
                 characters, which changes no host and no key"
            );
            Some(String::from_utf8_lossy(e.as_bytes()).into_owned())
        }
    }
}

// ---- the small primitives --------------------------------------------

/// A base64 field of a known_hosts line, read the way libssh2 reads it:
/// padding optional, and nothing else in the alphabet.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let digits: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '/')
        .collect();
    base64ct::Base64Unpadded::decode_vec(&digits).ok()
}

/// `HMAC-SHA1(salt, name)`, the hash a hashed known_hosts entry stores
/// (knownhost.c:416-441).
fn hmac_sha1(salt: &[u8], name: &[u8]) -> Vec<u8> {
    use hmac::Mac;
    let mut mac =
        hmac::Hmac::<sha1::Sha1>::new_from_slice(salt).expect("HMAC takes a key of any length");
    mac.update(name);
    mac.finalize().into_bytes().to_vec()
}

/// `*`, `?` and literal text, as man sshd defines a pattern. Host names
/// are compared without case, which is what ssh does with the name it
/// dialled.
fn wildcard_match(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.to_ascii_lowercase().chars().collect();
    let name: Vec<char> = name.to_ascii_lowercase().chars().collect();
    // the classic two index walk with one remembered star, which needs
    // no allocation per step and cannot go quadratic on a pattern a
    // person would write
    let (mut p, mut n) = (0usize, 0usize);
    let (mut star, mut resume) = (None, 0usize);
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            resume = n;
            p += 1;
        } else if let Some(at) = star {
            p = at + 1;
            resume += 1;
            n = resume;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

// ---- the pinned host keys of the three public forges ------------------

pub mod pins;

#[cfg(test)]
mod tests;
