// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The ssh credential chain (design D1.4).
//!
//! libgit2 offers ssh credentials one after another and re-enters the
//! credentials callback while the answer is `GIT_EAUTH`, so several
//! candidates cost ONE contact. What it does not do is tell anybody
//! apart: "there is no agent", "the agent holds no identity" and "the
//! agent was refused" all arrive as `GIT_EAUTH` with "error
//! authenticating" (ssh_libssh2.c:236-290). joy therefore looks at the
//! agent itself, before the contact, so that each of the three has its
//! own sentence.
//!
//! Key files are checked before they are handed over, and that check is
//! not cosmetic: a key file libssh2 cannot read is `LIBSSH2_ERROR_FILE`,
//! which libgit2 turns into -1 and which ends the WHOLE operation
//! instead of moving on to the next candidate (ssh_libssh2.c:366-380).
//! One passphrase-protected key in `~/.ssh` would otherwise stop every
//! fetch, and on Windows every modern key would, because WinCNG reads
//! no `openssh-key-v1` file at all (wincng.c:887-942).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::host_kind::HostKind;
use super::ssh_config::HostSettings;

/// How long joy waits for the agent to answer its identity list. The
/// agent is a local socket; anything slower than this is not there.
const AGENT_TIMEOUT: Duration = Duration::from_secs(2);

const REQUEST_IDENTITIES: u8 = 11;
const IDENTITIES_ANSWER: u8 = 12;

/// What the agent probe found. Three states, three sentences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Agent {
    /// No agent: nothing points at one.
    Missing,
    /// Something points at an agent that does not answer.
    Unreachable { socket: String, detail: String },
    /// An agent, with no identity in it.
    Empty { socket: String },
    /// An agent with identities to offer.
    Ready { socket: String, identities: u32 },
}

impl Agent {
    /// Whether it is worth offering the agent to libgit2 at all.
    pub fn usable(&self) -> bool {
        matches!(self, Agent::Ready { .. })
    }

    /// What a person is told about this agent and `host`.
    pub fn sentence(&self, host: &str) -> String {
        match self {
            Agent::Missing => format!(
                "no ssh agent is running (SSH_AUTH_SOCK names none), so joy has no agent identity to offer {host}"
            ),
            Agent::Unreachable { socket, detail } => format!(
                "the ssh agent at {socket} did not answer ({detail}), so joy has no agent identity to offer {host}"
            ),
            Agent::Empty { socket } => format!(
                "the ssh agent at {socket} holds no identity, so joy has nothing to offer {host}"
            ),
            Agent::Ready { identities, .. } => format!(
                "the ssh agent offered {identities} identities and {host} refused every one of them"
            ),
        }
    }
}

/// Ask the agent how many identities it holds, without contacting the
/// forge.
pub fn probe_agent() -> Agent {
    let Some(socket) = std::env::var("SSH_AUTH_SOCK")
        .ok()
        .filter(|s| !s.is_empty())
    else {
        return Agent::Missing;
    };
    match connect(&socket) {
        Ok(mut stream) => match identity_count(&mut stream) {
            Ok(0) => Agent::Empty { socket },
            Ok(identities) => Agent::Ready { socket, identities },
            Err(detail) => Agent::Unreachable { socket, detail },
        },
        Err(detail) => Agent::Unreachable { socket, detail },
    }
}

#[cfg(unix)]
fn connect(socket: &str) -> Result<std::os::unix::net::UnixStream, String> {
    let stream = std::os::unix::net::UnixStream::connect(socket).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(AGENT_TIMEOUT)).ok();
    stream.set_write_timeout(Some(AGENT_TIMEOUT)).ok();
    Ok(stream)
}

/// On Windows the agent is a named pipe, which opens like a file.
#[cfg(windows)]
fn connect(socket: &str) -> Result<std::fs::File, String> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(socket)
        .map_err(|e| e.to_string())
}

/// The agent protocol, as far as joy needs it: ask for the identity
/// list, read how long it is.
fn identity_count(stream: &mut (impl Read + Write)) -> Result<u32, String> {
    stream
        .write_all(&[0, 0, 0, 1, REQUEST_IDENTITIES])
        .map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;
    let mut header = [0u8; 5];
    stream.read_exact(&mut header).map_err(|e| e.to_string())?;
    let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
    if header[4] != IDENTITIES_ANSWER {
        return Err(format!("the agent answered with message {}", header[4]));
    }
    if length < 5 {
        return Err("the agent answered with a truncated identity list".to_string());
    }
    let mut count = [0u8; 4];
    stream.read_exact(&mut count).map_err(|e| e.to_string())?;
    Ok(u32::from_be_bytes(count))
}

/// The shape of a private key file, as far as reading it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyFormat {
    /// `-----BEGIN OPENSSH PRIVATE KEY-----`: what ssh-keygen has
    /// written by default since 2017, and what WinCNG cannot read.
    OpenSshV1,
    /// `-----BEGIN RSA PRIVATE KEY-----`: classic PKCS#1, the one
    /// format that works on every backend joy links.
    Pkcs1Rsa,
    /// Another classic PEM key (DSA, EC).
    ClassicPem,
    /// PKCS#8, plain or encrypted.
    Pkcs8,
    /// PuTTY's own format, which no backend joy links reads.
    Putty,
    /// Something else, or not a key at all.
    Unknown,
}

/// A key file joy looked at before offering it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyFile {
    pub path: PathBuf,
    pub format: KeyFormat,
    pub encrypted: bool,
}

/// Read the header of a key file. Only the first few hundred bytes are
/// needed, but a key file is small enough to read whole.
pub fn examine(path: &Path) -> std::io::Result<KeyFile> {
    let text = std::fs::read_to_string(path)?;
    Ok(examine_text(path, &text))
}

/// [`examine`] on text that is already in hand (and the seam the tests
/// use).
pub fn examine_text(path: &Path, text: &str) -> KeyFile {
    let head = text.trim_start();
    let (format, encrypted) = if head.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----") {
        (KeyFormat::OpenSshV1, openssh_is_encrypted(text))
    } else if head.starts_with("-----BEGIN RSA PRIVATE KEY-----") {
        (KeyFormat::Pkcs1Rsa, has_proc_type(text))
    } else if head.starts_with("-----BEGIN ENCRYPTED PRIVATE KEY-----") {
        (KeyFormat::Pkcs8, true)
    } else if head.starts_with("-----BEGIN PRIVATE KEY-----") {
        (KeyFormat::Pkcs8, false)
    } else if head.starts_with("PuTTY-User-Key-File") {
        (KeyFormat::Putty, text.contains("Encryption: aes"))
    } else if head.starts_with("-----BEGIN") && head.contains("PRIVATE KEY-----") {
        (KeyFormat::ClassicPem, has_proc_type(text))
    } else {
        (KeyFormat::Unknown, false)
    };
    KeyFile {
        path: path.to_path_buf(),
        format,
        encrypted,
    }
}

/// The classic PEM way of saying "encrypted" (RFC 1421 headers).
fn has_proc_type(text: &str) -> bool {
    text.lines()
        .take(6)
        .any(|line| line.trim().starts_with("Proc-Type:") && line.contains("ENCRYPTED"))
}

/// An `openssh-key-v1` blob names its cipher in clear text right after
/// the magic: `none` means the key is not encrypted.
fn openssh_is_encrypted(text: &str) -> bool {
    use base64ct::Encoding;
    let body: String = text
        .lines()
        .skip_while(|line| !line.starts_with("-----BEGIN"))
        .skip(1)
        .take_while(|line| !line.starts_with("-----END"))
        .collect::<Vec<_>>()
        .join("");
    let Ok(blob) = base64ct::Base64::decode_vec(body.trim()) else {
        // Unreadable base64: treat it as encrypted, which means
        // "skipped with a reason" rather than "handed to libssh2 and
        // the whole fetch dies".
        return true;
    };
    const MAGIC: &[u8] = b"openssh-key-v1\0";
    if !blob.starts_with(MAGIC) {
        return true;
    }
    let rest = &blob[MAGIC.len()..];
    if rest.len() < 4 {
        return true;
    }
    let length = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
    let Some(cipher) = rest.get(4..4 + length) else {
        return true;
    };
    cipher != b"none"
}

/// One candidate the chain offers to libgit2, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshCandidate {
    /// The agent, which joy has already found to hold identities.
    Agent,
    /// A key file joy has read the header of.
    Key {
        path: PathBuf,
        /// The matching `.pub`, when there is one. libssh2 can derive
        /// it, but handing it over saves the derivation.
        public: Option<PathBuf>,
        /// The passphrase, when the key needs one and a person gave it.
        passphrase: Option<String>,
    },
}

/// The chain for one host: what to offer, in which order, under which
/// user name, and what to say about everything that was left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshChain {
    /// The user name, decided ONCE. libgit2 re-enters the callback with
    /// the same user name it was first given (ssh_libssh2.c:865); a
    /// chain that changes it mid-flight authenticates as somebody else
    /// halfway through.
    pub user: String,
    pub candidates: Vec<SshCandidate>,
    /// Why a candidate is not in the list, in the person's words.
    pub notes: Vec<String>,
}

/// Build the chain for `host`.
///
/// `windows` is a parameter and not a `cfg!`, so that the rule which
/// only bites on Windows is testable everywhere.
pub fn chain_for(
    host: &str,
    url_user: Option<&str>,
    settings: &HostSettings,
    kind: HostKind,
    agent: &Agent,
    windows: bool,
) -> SshChain {
    let user = settings
        .user
        .clone()
        .or_else(|| url_user.map(str::to_string))
        .unwrap_or_else(|| "git".to_string());
    let mut candidates = Vec::new();
    let mut notes = Vec::new();
    if agent.usable() {
        candidates.push(SshCandidate::Agent);
    } else {
        notes.push(agent.sentence(host));
    }
    let identity_files = if settings.identity_files.is_empty() {
        default_identity_files()
    } else {
        settings.identity_files.clone()
    };
    for path in identity_files {
        if !path.is_file() {
            continue;
        }
        let key = match examine(&path) {
            Ok(key) => key,
            Err(e) => {
                notes.push(format!("key {}: cannot be read ({e})", path.display()));
                continue;
            }
        };
        match candidate_for(&key, kind, windows) {
            Ok(candidate) => candidates.push(candidate),
            Err(note) => notes.push(note),
        }
    }
    SshChain {
        user,
        candidates,
        notes,
    }
}

/// One key file's verdict: a candidate, or the sentence that says why
/// it is not one.
pub fn candidate_for(key: &KeyFile, kind: HostKind, windows: bool) -> Result<SshCandidate, String> {
    let path = key.path.display();
    if windows && key.format != KeyFormat::Pkcs1Rsa {
        // On Windows libssh2 reads keys through WinCNG, which loads a
        // classic PEM RSA key and nothing else: it does not read an
        // openssh-key-v1 file at all (wincng.c:887-942). Named, never
        // silent, because on Windows this is the normal state of every
        // key ssh-keygen has written since 2017.
        return Err(match key.format {
            KeyFormat::OpenSshV1 => format!(
                "key {path}: this key file is in the OpenSSH format, which joy cannot read on Windows"
            ),
            _ => format!(
                "key {path}: joy can read only a classic PKCS#1 RSA key file on Windows, and this is not one"
            ),
        });
    }
    match key.format {
        KeyFormat::Putty => {
            return Err(format!(
                "key {path}: this key file is in PuTTY's format, which joy cannot read; export it as OpenSSH"
            ))
        }
        KeyFormat::Unknown => {
            return Err(format!(
                "key {path}: joy does not recognise this file as a private key"
            ))
        }
        _ => {}
    }
    let mut passphrase = None;
    if key.encrypted {
        match ask_passphrase(&key.path, kind) {
            Some(secret) => passphrase = Some(secret),
            // The exact words of D1.4 for a host that cannot ask.
            None => return Err(format!("key {path}: passphrase needed, skipped")),
        }
    }
    let public = {
        let candidate = key.path.with_extension("pub");
        candidate.is_file().then_some(candidate)
    };
    Ok(SshCandidate::Key {
        path: key.path.clone(),
        public,
        passphrase,
    })
}

/// ssh's own default identity files, in ssh's own order.
fn default_identity_files() -> Vec<PathBuf> {
    let Some(home) = home_ssh_dir() else {
        return Vec::new();
    };
    [
        "id_ed25519",
        "id_ecdsa",
        "id_ecdsa_sk",
        "id_ed25519_sk",
        "id_rsa",
        "id_dsa",
    ]
    .iter()
    .map(|name| home.join(name))
    .collect()
}

fn home_ssh_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    let home = std::env::var_os("HOME");
    #[cfg(not(unix))]
    let home = std::env::var_os("USERPROFILE");
    home.filter(|h| !h.is_empty())
        .map(|home| PathBuf::from(home).join(".ssh"))
}

// ---- the passphrase question ------------------------------------------

type Ask = Arc<dyn Fn(&Path) -> Option<String> + Send + Sync>;

static PROMPT: Mutex<Option<Ask>> = Mutex::new(None);
static ANSWERS: Mutex<Option<std::collections::HashMap<PathBuf, Option<String>>>> =
    Mutex::new(None);

/// Install the question joy asks when a key file needs a passphrase.
///
/// The host installs it, and only a host where a person is sitting
/// does: joy-core has no terminal and no window of its own. A host that
/// installs none, and every `Background` and `Delegated` host whatever
/// it installed, skips the key by name instead.
pub fn set_passphrase_prompt(ask: impl Fn(&Path) -> Option<String> + Send + Sync + 'static) {
    *PROMPT.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(ask));
}

/// Take the question back, and forget every answer. For a host that
/// changes mode, and for the tests.
pub fn clear_passphrase_prompt() {
    *PROMPT.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *ANSWERS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The passphrase for `path`: asked at most ONCE per process per key,
/// and only of a host that has somebody to ask. The answer, including
/// "the person said no", is kept for the run.
fn ask_passphrase(path: &Path, kind: HostKind) -> Option<String> {
    if !kind.may_prompt() {
        return None;
    }
    let mut answers = ANSWERS.lock().unwrap_or_else(|e| e.into_inner());
    let answers = answers.get_or_insert_with(std::collections::HashMap::new);
    if let Some(known) = answers.get(path) {
        return known.clone();
    }
    let ask = PROMPT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .cloned();
    let answer = ask.and_then(|ask| ask(path)).filter(|s| !s.is_empty());
    answers.insert(path.to_path_buf(), answer.clone());
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The passphrase question and its answers are process-wide, so
    /// the tests that touch them run one at a time.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn write_key(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    /// An `openssh-key-v1` blob whose cipher field says `cipher`.
    fn openssh_key(cipher: &str) -> String {
        use base64ct::Encoding;
        let mut blob = b"openssh-key-v1\0".to_vec();
        blob.extend((cipher.len() as u32).to_be_bytes());
        blob.extend(cipher.as_bytes());
        blob.extend((4u32).to_be_bytes());
        blob.extend(b"none");
        let body = base64ct::Base64::encode_string(&blob);
        format!("-----BEGIN OPENSSH PRIVATE KEY-----\n{body}\n-----END OPENSSH PRIVATE KEY-----\n")
    }

    #[test]
    fn the_three_agent_states_are_three_sentences() {
        let host = "github.com";
        let missing = Agent::Missing.sentence(host);
        let empty = Agent::Empty {
            socket: "/run/agent".into(),
        }
        .sentence(host);
        let refused = Agent::Ready {
            socket: "/run/agent".into(),
            identities: 2,
        }
        .sentence(host);
        assert!(missing.contains("no ssh agent is running"), "{missing}");
        assert!(empty.contains("holds no identity"), "{empty}");
        assert!(refused.contains("refused every one of them"), "{refused}");
        assert_ne!(missing, empty);
        assert_ne!(empty, refused);
        assert!(!Agent::Missing.usable());
        assert!(!Agent::Empty { socket: "s".into() }.usable());
        assert!(Agent::Ready {
            socket: "s".into(),
            identities: 1
        }
        .usable());
    }

    #[test]
    fn the_agent_protocol_reads_the_identity_count() {
        // The agent's answer: length 5 + 4, type 12, count 3.
        struct Fake {
            written: Vec<u8>,
            answer: std::io::Cursor<Vec<u8>>,
        }
        impl Write for Fake {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.written.extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl Read for Fake {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                self.answer.read(buf)
            }
        }
        let mut answer = vec![0, 0, 0, 9, IDENTITIES_ANSWER];
        answer.extend(3u32.to_be_bytes());
        let mut fake = Fake {
            written: Vec::new(),
            answer: std::io::Cursor::new(answer),
        };
        assert_eq!(identity_count(&mut fake).unwrap(), 3);
        assert_eq!(fake.written, vec![0, 0, 0, 1, REQUEST_IDENTITIES]);
    }

    #[test]
    fn a_key_files_format_and_lock_are_read_from_its_header() {
        let dir = tempfile::tempdir().unwrap();
        let plain = examine_text(Path::new("k"), &openssh_key("none"));
        assert_eq!(plain.format, KeyFormat::OpenSshV1);
        assert!(!plain.encrypted);
        let locked = examine_text(Path::new("k"), &openssh_key("aes256-ctr"));
        assert!(locked.encrypted);
        let classic = examine_text(
            Path::new("k"),
            "-----BEGIN RSA PRIVATE KEY-----\nMIIB\n-----END RSA PRIVATE KEY-----\n",
        );
        assert_eq!(classic.format, KeyFormat::Pkcs1Rsa);
        assert!(!classic.encrypted);
        let classic_locked = examine_text(
            Path::new("k"),
            "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nDEK-Info: AES-128-CBC,1\n\nMIIB\n-----END RSA PRIVATE KEY-----\n",
        );
        assert!(classic_locked.encrypted);
        let pkcs8 = examine_text(
            Path::new("k"),
            "-----BEGIN ENCRYPTED PRIVATE KEY-----\nMIIB\n-----END ENCRYPTED PRIVATE KEY-----\n",
        );
        assert_eq!(pkcs8.format, KeyFormat::Pkcs8);
        assert!(pkcs8.encrypted);
        let from_disk =
            examine(&write_key(dir.path(), "id_ed25519", &openssh_key("none"))).unwrap();
        assert_eq!(from_disk.format, KeyFormat::OpenSshV1);
    }

    #[test]
    fn a_passphrase_protected_key_is_skipped_by_name_and_never_aborts_the_chain() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        clear_passphrase_prompt();
        let key = examine_text(
            Path::new("/home/p/.ssh/id_ed25519"),
            &openssh_key("aes256-ctr"),
        );
        let note = candidate_for(&key, HostKind::Background, false).unwrap_err();
        assert_eq!(
            note,
            "key /home/p/.ssh/id_ed25519: passphrase needed, skipped"
        );
        let delegated = candidate_for(&key, HostKind::Delegated, false).unwrap_err();
        assert!(delegated.ends_with("passphrase needed, skipped"));
    }

    #[test]
    fn on_windows_an_openssh_key_is_reported_by_name() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        clear_passphrase_prompt();
        let key = examine_text(
            Path::new("C:\\Users\\p\\.ssh\\id_ed25519"),
            &openssh_key("none"),
        );
        let note = candidate_for(&key, HostKind::Interactive, true).unwrap_err();
        assert_eq!(
            note,
            "key C:\\Users\\p\\.ssh\\id_ed25519: this key file is in the OpenSSH format, which joy cannot read on Windows"
        );
        // The same key is offered everywhere else.
        assert!(candidate_for(&key, HostKind::Interactive, false).is_ok());
        // And the classic PKCS#1 key is the one that still works there.
        let classic = examine_text(
            Path::new("C:\\Users\\p\\.ssh\\id_rsa"),
            "-----BEGIN RSA PRIVATE KEY-----\nMIIB\n-----END RSA PRIVATE KEY-----\n",
        );
        assert!(candidate_for(&classic, HostKind::Interactive, true).is_ok());
        // Every other format is named too, rather than offered to a
        // backend that cannot read it.
        let ec = examine_text(
            Path::new("C:\\Users\\p\\.ssh\\id_ecdsa"),
            "-----BEGIN EC PRIVATE KEY-----\nMIIB\n-----END EC PRIVATE KEY-----\n",
        );
        assert!(candidate_for(&ec, HostKind::Interactive, true)
            .unwrap_err()
            .contains("classic PKCS#1 RSA key file on Windows"));
        assert!(candidate_for(&ec, HostKind::Interactive, false).is_ok());
    }

    #[test]
    fn an_interactive_host_is_asked_once_per_key_and_a_quiet_one_never() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        clear_passphrase_prompt();
        let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = asked.clone();
        set_passphrase_prompt(move |_path| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some("open sesame".to_string())
        });
        let key = examine_text(
            Path::new("/home/p/.ssh/id_ed25519"),
            &openssh_key("aes256-ctr"),
        );
        for _ in 0..3 {
            match candidate_for(&key, HostKind::Interactive, false).unwrap() {
                SshCandidate::Key { passphrase, .. } => {
                    assert_eq!(passphrase.as_deref(), Some("open sesame"))
                }
                other => panic!("expected a key, got {other:?}"),
            }
        }
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);
        // A quiet host never reaches the question, however it is
        // installed.
        assert!(candidate_for(&key, HostKind::Background, false).is_err());
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);
        clear_passphrase_prompt();
    }

    #[test]
    fn the_chain_offers_the_agent_first_then_every_key_that_survived() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        clear_passphrase_prompt();
        let dir = tempfile::tempdir().unwrap();
        let good = write_key(dir.path(), "id_ed25519", &openssh_key("none"));
        std::fs::write(dir.path().join("id_ed25519.pub"), "ssh-ed25519 AAAA\n").unwrap();
        let locked = write_key(dir.path(), "id_locked", &openssh_key("aes256-ctr"));
        let settings = HostSettings {
            user: Some("deploy".into()),
            identity_files: vec![good.clone(), locked.clone(), dir.path().join("absent")],
            ..HostSettings::default()
        };
        let chain = chain_for(
            "git.example.com",
            Some("fromurl"),
            &settings,
            HostKind::Background,
            &Agent::Ready {
                socket: "/run/agent".into(),
                identities: 1,
            },
            false,
        );
        // The config's user wins over the URL's, and it is decided once.
        assert_eq!(chain.user, "deploy");
        assert_eq!(chain.candidates.len(), 2);
        assert_eq!(chain.candidates[0], SshCandidate::Agent);
        assert_eq!(
            chain.candidates[1],
            SshCandidate::Key {
                path: good,
                public: Some(dir.path().join("id_ed25519.pub")),
                passphrase: None
            }
        );
        // The locked key is named, the absent one is not a story.
        assert_eq!(chain.notes.len(), 1);
        assert!(chain.notes[0].contains("passphrase needed, skipped"));
        assert!(chain.notes[0].contains(&locked.display().to_string()));
    }

    #[test]
    fn without_an_agent_the_chain_says_so_and_still_offers_the_keys() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        clear_passphrase_prompt();
        let dir = tempfile::tempdir().unwrap();
        let good = write_key(dir.path(), "id_ed25519", &openssh_key("none"));
        let settings = HostSettings {
            identity_files: vec![good],
            ..HostSettings::default()
        };
        let chain = chain_for(
            "github.com",
            None,
            &settings,
            HostKind::Background,
            &Agent::Missing,
            false,
        );
        assert_eq!(chain.user, "git");
        assert_eq!(chain.candidates.len(), 1);
        assert!(chain.notes[0].contains("no ssh agent is running"));
    }
}
