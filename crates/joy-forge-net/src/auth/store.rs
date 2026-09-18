// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The connector's own credential entry (D2.6).
//!
//! Who may read which entry is decided and narrow: **the connector
//! binary owns the entry and nobody else touches it**. The app and the
//! CLI call this same signed binary for `login`, `token`, `token-store`
//! and `logout`. On macOS that is the only arrangement without a
//! confirmation dialog, because a generic password is created with an
//! ACL that trusts the creating application alone.
//!
//! Three decisions of D2.6 are load bearing here:
//!
//! - **`Entry::new` only, never `new_with_target`.** The service is the
//!   connector's own name, the user is `<host>` or `<host>|<login>`.
//! - **joy does not read a foreign CLI's store at all.** Reading gh's
//!   or glab's item would need `new_with_target` on Windows and risks
//!   an allow-or-deny dialog on macOS, so a foreign credential is
//!   obtained by SPAWNING that CLI (`crate::foreign`), which is also
//!   the only way its own refresh runs.
//! - **A fallback the crate does not have.** On `NoStorageAccess`, on
//!   `PlatformFailure` and on any target where the crate would degrade
//!   to its in process mock, joy writes `forge-tokens.json` itself,
//!   mode 0600 in a 0700 directory, and says so. gh's own sentence is
//!   the model: "If a credential store is not found or there is an
//!   issue using it gh will fallback to writing the token to a plain
//!   text file".
//!
//! The mock case is detected rather than guessed: after a write the
//! entry is read back through a NEW `Entry`, and a store that cannot
//! answer with what was just written did not persist it. keyring's mock
//! keeps the password inside the `Entry` object itself
//! (`CredentialPersistence::EntryOnly`), so it fails exactly that read.
//!
//! Two rules hold for the file, and neither is optional:
//!
//! - **One writer at a time.** The file is ONE document for every host
//!   and every login, while the refresh lock of D2.6a is per host and
//!   login: two `login` runs for two logins on one host take two
//!   different locks. Every read-modify-write of the file therefore
//!   takes one more whole file lock, on the same primitive, beside the
//!   document it protects. Without it the second writer's document
//!   predates the first writer's commit and one login's entry is
//!   silently gone.
//! - **A write is a rename.** The document is written to a staging file
//!   in the same directory and renamed over the target, so no reader
//!   ever sees half of it. A truncate-then-write would make a crash mid
//!   write lose every stored credential at once, because a file that
//!   does not parse reads as "nothing is stored".
//!
//! One more rule comes from outside this module. A `Delegated` host is
//! an agent the person lent their machine to, and G2 says it inherits
//! everything through the joy CLI: it may READ the credential the
//! person stored and it may never change it. Its vault is therefore
//! built read only ([`Vault::read_only`]), which is a refusal in
//! [`Vault::put`] and [`Vault::remove`] and a "do not renew" in the
//! refresh of D2.6a. Nothing about that decision is per call: the flag
//! is set once, where the host kind is known (`Ctx::new`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::Source;

/// The service name of the connector's own entries. It is the binary's
/// own name, as D2.6 says, and never a forge's.
pub const SERVICE: &str = "joy-forge";

/// The service name of the per host login index. `Entry::new` cannot
/// enumerate, and D4.1c's "the only login the host holds" and its probe
/// candidate order both need the list, so the list is an entry of its
/// own under a service that no `<host>|<login>` user can collide with.
pub const INDEX_SERVICE: &str = "joy-forge.logins";

/// The file joy writes when the credential store cannot answer (D2.6).
pub const FALLBACK_FILE: &str = "forge-tokens.json";

/// Why a read only vault refused a write. It reaches a person only
/// through a caller that says so in its own words; the verbs of D2.4
/// refuse before they get this far.
pub const READ_ONLY: &str =
    "this process runs under a delegation session, which may use the credential this machine \
     holds and may never change it";

/// One stored credential. Everything the refresh and the answers of
/// D2.4 need is here, so a refresh needs no forge knowledge at all:
/// `token_endpoint` and `client_id` are the ones the login used.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The access token. The only secret in the record.
    pub token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// The granted scope set, space separated, beside the token in the
    /// same entry (D2.7c). GitHub and GitLab report it; Gitea's token
    /// answer has no scope field, so there the connector stores the set
    /// it requested.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scopes: String,
    /// RFC 3339, when the forge gave a lifetime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Present only where the forge issued one. Forgejo rotates it on
    /// every use, which is why a refresh writes the whole record back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
}

/// The record prints its FINGERPRINT and never its token.
///
/// Rule 1 of this module's header is that a secret never reaches a log
/// line, and `joy_core::forge_plugins::ForgeToken` keeps that rule by
/// having no `Debug` at all. This type has to stay printable, because a
/// failing test must be able to say which record it compared, so the
/// two fields that must not travel are replaced by the twelve hex
/// digits D2.6a already compares under the lock: they identify a token
/// without carrying it. One `tracing::debug!(?record)` is therefore
/// safe by construction and not by discipline.
impl std::fmt::Debug for Record {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Record")
            .field("token", &super::Redacted(&self.token))
            .field("login", &self.login)
            .field("user_id", &self.user_id)
            .field("scopes", &self.scopes)
            .field("expires_at", &self.expires_at)
            .field(
                "refresh_token",
                &self.refresh_token.as_deref().map(super::Redacted),
            )
            .field("token_endpoint", &self.token_endpoint)
            .field("client_id", &self.client_id)
            .finish()
    }
}

impl Record {
    /// Whether this token is past its lifetime, measured against the
    /// 60 s skew of D2.6a.
    pub fn is_expired(&self) -> bool {
        self.expires_in().is_some_and(|left| left <= 60)
    }

    /// Seconds left on this token, `None` when the forge named no
    /// lifetime (a gh style `gho_` token, a personal access token).
    pub fn expires_in(&self) -> Option<i64> {
        let at = self.expires_at.as_deref()?;
        let at = chrono::DateTime::parse_from_rfc3339(at).ok()?;
        Some((at.timestamp() - chrono::Utc::now().timestamp()).max(0))
    }

    /// Whether a refresh is possible at all: the forge issued a refresh
    /// token and named where to spend it.
    pub fn can_refresh(&self) -> bool {
        self.refresh_token.is_some() && self.token_endpoint.is_some()
    }

    /// The fingerprint D2.6a compares under the lock.
    pub fn fingerprint(&self) -> String {
        super::fingerprint(&self.token)
    }
}

/// Where the connector keeps its credentials. One per process; the
/// decision between the credential store and the file is made per
/// operation, because a store can be there and still refuse.
#[derive(Debug, Clone)]
pub struct Vault {
    /// The credential store this vault may ask.
    keys: Keys,
    /// The 0600 file, used when the store cannot answer.
    file: PathBuf,
    /// Whether this vault may only be read. A `Delegated` host gets
    /// one that may not: it uses what the person stored and changes
    /// nothing of it (D1.10, D3.8).
    read_only: bool,
}

/// The credential store behind a vault.
#[derive(Clone, Default)]
enum Keys {
    /// None at all: the 0600 file alone, or nothing.
    #[default]
    None,
    /// The operating system's, through `keyring::Entry::new` and
    /// nothing else (D2.6).
    Os,
    /// An in process store with the same semantics an operating
    /// system's one has: it persists across `Entry` objects, which is
    /// exactly the property keyring's own mock lacks and which the read
    /// back of [`Keys::put_checked`] tests for. Behind `fake-api`, so a
    /// shipped connector never carries it, and it is what lets a test
    /// execute the keychain half of D2.6 at all: every test that drove
    /// the file alone left the entry addressing, the login index and
    /// the mock detector unproven.
    #[cfg(feature = "fake-api")]
    Fake(std::sync::Arc<std::sync::Mutex<BTreeMap<String, String>>>),
}

/// Rule 1 of this module's parent: the in process store of a test holds
/// real token text, so it prints how many entries it has and never what
/// is in them.
impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Keys::None => write!(f, "Keys::None"),
            Keys::Os => write!(f, "Keys::Os"),
            #[cfg(feature = "fake-api")]
            Keys::Fake(store) => write!(
                f,
                "Keys::Fake({} entries)",
                store.lock().unwrap_or_else(|e| e.into_inner()).len()
            ),
        }
    }
}

impl Keys {
    /// One entry read, through a NEW handle every time. Never
    /// `new_with_target`, and never a foreign service: this reads what
    /// this binary wrote and nothing else.
    fn get(&self, service: &str, user: &str) -> Option<String> {
        match self {
            Keys::None => None,
            // `NoEntry` is the normal answer for a host nobody signed
            // in to; `NoStorageAccess` and `PlatformFailure` are a
            // store that could not answer. Both end here, and the
            // caller falls through to the 0600 file of D2.6.
            Keys::Os => {
                let (service, user) = (service.to_string(), user.to_string());
                bounded(move || {
                    keyring::Entry::new(&service, &user)
                        .ok()?
                        .get_password()
                        .ok()
                })
                .flatten()
            }
            #[cfg(feature = "fake-api")]
            Keys::Fake(store) => store
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&fake_key(service, user))
                .cloned(),
        }
    }

    fn set(&self, service: &str, user: &str, secret: &str) -> bool {
        match self {
            Keys::None => false,
            Keys::Os => {
                let (service, user, secret) =
                    (service.to_string(), user.to_string(), secret.to_string());
                bounded(move || {
                    keyring::Entry::new(&service, &user)
                        .is_ok_and(|entry| entry.set_password(&secret).is_ok())
                })
                .unwrap_or(false)
            }
            #[cfg(feature = "fake-api")]
            Keys::Fake(store) => {
                store
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(fake_key(service, user), secret.to_string());
                true
            }
        }
    }

    fn delete(&self, service: &str, user: &str) -> bool {
        match self {
            Keys::None => false,
            Keys::Os => {
                let (service, user) = (service.to_string(), user.to_string());
                bounded(move || {
                    keyring::Entry::new(&service, &user)
                        .is_ok_and(|entry| entry.delete_credential().is_ok())
                })
                .unwrap_or(false)
            }
            #[cfg(feature = "fake-api")]
            Keys::Fake(store) => store
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&fake_key(service, user))
                .is_some(),
        }
    }

    /// One entry written, then read back through a NEW handle.
    ///
    /// The read back is the mock detector of D2.6: keyring's in process
    /// mock keeps the password inside the `Entry` object, so a second
    /// entry for the same service and user finds nothing, and joy must
    /// write its own file instead of believing a store that stored
    /// nothing.
    fn put_checked(&self, service: &str, user: &str, secret: &str) -> bool {
        if !self.set(service, user, secret) {
            return false;
        }
        self.get(service, user).as_deref() == Some(secret)
    }
}

/// How long one call into the operating system's store may take before
/// it counts as a store that could not answer.
///
/// The Secret Service unlocks a locked collection by PROMPTING, and the
/// prompt is drawn by the desktop session, not by the caller. On a
/// machine whose keyring is locked and which has no desktop session to
/// draw the prompt (an ssh login, a headless box, a session whose
/// prompter died) that call never returns: on 2026-09-18 a `login` sat
/// in the write after GitHub had already granted the token, until the
/// device code expired and the grant was lost (JOY-02AA-ED). D2.6 says
/// a store that cannot answer is the file's turn, and a store that does
/// not answer in this time is that store. The bound is well under the
/// 20 seconds joy gives the connector as a whole (D1.9a), so a locked
/// keyring costs one wait per process and never a "plugin did not
/// answer" on top.
pub const KEYRING_BOUND: std::time::Duration = std::time::Duration::from_secs(5);

/// Whether a call into the store already failed to answer in this
/// process. One wait is the price of finding out; a second call into
/// the same stuck prompt would pay it again for the same answer, so
/// after the first silence the store is skipped for the rest of the
/// process and every read and write goes to the file.
static KEYRING_SILENT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Run one store call on its own thread and wait [`KEYRING_BOUND`] for
/// it. `None` is the store not answering; the thread is left behind
/// with its prompt, which the process end collects. The silence is
/// remembered (see [`KEYRING_SILENT`]) so the next call does not wait.
fn bounded<T: Send + 'static>(call: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    use std::sync::atomic::Ordering;
    if KEYRING_SILENT.load(Ordering::Relaxed) {
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(call());
    });
    match rx.recv_timeout(KEYRING_BOUND) {
        Ok(answer) => Some(answer),
        Err(_) => {
            KEYRING_SILENT.store(true, Ordering::Relaxed);
            None
        }
    }
}

/// The key of one entry in the in process store, which addresses an
/// entry by service and user exactly as `Entry::new` does.
#[cfg(feature = "fake-api")]
fn fake_key(service: &str, user: &str) -> String {
    format!("{service}\n{user}")
}

impl Vault {
    /// The real vault: the credential store first, joy's own file
    /// behind it.
    pub fn real() -> Self {
        Vault {
            keys: Keys::Os,
            file: default_file(),
            read_only: false,
        }
    }

    /// The same vault, allowed to read and nothing else.
    ///
    /// This is what a `Delegated` host gets. The agent runs on the
    /// person's own machine and inherits their credentials through the
    /// joy CLI (G2, D3.8), so refusing it the store would refuse it
    /// every ssh and https contact the person can make. What it may
    /// never do is CHANGE what it inherited: no `token-store`, no
    /// `logout`, and no refresh, because a refresh at a forge that
    /// rotates refresh tokens retires the one the person holds and
    /// signs them out of their own machine. Where a refresh would be
    /// needed the answer is `known:false` with the sentence that names
    /// who has to sign in (D1.10: a delegated host never prompts).
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// A vault that only ever uses the file under `dir`. This is the
    /// fallback path of D2.6 on purpose, and it is what most tests
    /// drive: a test must never write into the person's own keychain,
    /// and on a machine with a real Secret Service it would.
    pub fn file_at(dir: impl AsRef<Path>) -> Self {
        Vault {
            keys: Keys::None,
            file: dir.as_ref().join(FALLBACK_FILE),
            read_only: false,
        }
    }

    /// A vault whose credential store is an in process one and whose
    /// file sits under `dir`: the shape [`Vault::real`] has on a
    /// machine with a working store, with both halves somewhere a test
    /// may write. Behind `fake-api`.
    #[cfg(feature = "fake-api")]
    pub fn fake_keychain_at(dir: impl AsRef<Path>) -> Self {
        Vault {
            keys: Keys::Fake(std::sync::Arc::new(std::sync::Mutex::new(BTreeMap::new()))),
            file: dir.as_ref().join(FALLBACK_FILE),
            read_only: false,
        }
    }

    /// A vault that asks the OPERATING SYSTEM's credential store and
    /// falls back to a file under `dir`. Behind `fake-api`, and only a
    /// test that has installed keyring's mock credential builder may
    /// build one: on a machine with a real Secret Service this writes
    /// into the person's own keychain.
    #[cfg(feature = "fake-api")]
    pub fn os_keychain_at(dir: impl AsRef<Path>) -> Self {
        Vault {
            keys: Keys::Os,
            file: dir.as_ref().join(FALLBACK_FILE),
            read_only: false,
        }
    }

    /// A vault that stores nothing. The pure verbs and every context a
    /// caller builds with [`crate::forge::Ctx::bare`] get this one, so
    /// no library call touches a credential store by accident.
    pub fn none() -> Self {
        Vault {
            keys: Keys::None,
            file: PathBuf::new(),
            read_only: false,
        }
    }

    /// Whether this vault can hold anything at all.
    pub fn is_none(&self) -> bool {
        matches!(self.keys, Keys::None) && self.file.as_os_str().is_empty()
    }

    /// Whether this vault may be read and not written.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// The file this vault falls back to.
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// The record for one host and login, and which store answered.
    pub fn get(&self, host: &str, login: Option<&str>) -> Option<(Record, Source)> {
        if self.is_none() {
            return None;
        }
        if let Some(text) = self.keys.get(SERVICE, &user_key(host, login)) {
            if let Ok(record) = serde_json::from_str::<Record>(&text) {
                return Some((record, Source::Keychain));
            }
        }
        let file = self.read_file();
        let record = file.get(host, login)?;
        Some((record, Source::File))
    }

    /// Every login this vault holds for a host, in insertion order.
    /// D4.1c's step 3 ("the only login the host holds, with no probe at
    /// all") is exactly this list having one entry.
    pub fn logins(&self, host: &str) -> Vec<String> {
        if self.is_none() {
            return Vec::new();
        }
        let mut logins: Vec<String> = Vec::new();
        if let Some(text) = self.keys.get(INDEX_SERVICE, host) {
            if let Ok(known) = serde_json::from_str::<Vec<String>>(&text) {
                logins.extend(known);
            }
        }
        for login in self.read_file().logins(host) {
            if !logins.contains(&login) {
                logins.push(login);
            }
        }
        logins
    }

    /// Store one record. Answers which store took it, so the caller can
    /// say `"stored":"keychain"` or `"stored":"file"` truthfully.
    pub fn put(&self, host: &str, record: &Record) -> Result<Source, String> {
        if self.is_none() {
            return Err("this connector call keeps no credentials".to_string());
        }
        if self.read_only {
            return Err(READ_ONLY.to_string());
        }
        let login = record.login.as_deref();
        let text = serde_json::to_string(record).map_err(|e| e.to_string())?;
        if self
            .keys
            .put_checked(SERVICE, &user_key(host, login), &text)
        {
            self.index_add(host, login);
            return Ok(Source::Keychain);
        }
        let guard = self.hold_file()?;
        let mut file = self.read_file_to_modify()?;
        file.put(host, record.clone());
        let written = self.write_file(&file);
        drop(guard);
        written?;
        Ok(Source::File)
    }

    /// Remove one record from wherever it is. Answers which store held
    /// it, `Ok(None)` when nothing was there, and an error when the
    /// entry is still there because joy could not write the file: D2.4
    /// answers `"removed": true`, so a read only or full state
    /// directory has to be a refusal and never a silent success.
    pub fn remove(&self, host: &str, login: Option<&str>) -> Result<Option<Source>, String> {
        if self.is_none() {
            return Ok(None);
        }
        if self.read_only {
            return Err(READ_ONLY.to_string());
        }
        let mut removed = None;
        if self.keys.delete(SERVICE, &user_key(host, login)) {
            self.index_remove(host, login);
            removed = Some(Source::Keychain);
        }
        let guard = match self.hold_file() {
            Ok(guard) => guard,
            // The credential store already gave the entry up, so the
            // call did what it said; the file half is reported as it
            // is, and a vault that has no file at all has nothing to
            // report.
            Err(message) if removed.is_some() => {
                eprintln!("joy: {message}");
                return Ok(removed);
            }
            Err(message) => return Err(message),
        };
        let mut file = match self.read_file_to_modify() {
            Ok(file) => file,
            // Same rule as above: the credential store already gave the
            // entry up, so the call did what it said.
            Err(message) if removed.is_some() => {
                eprintln!("joy: {message}");
                return Ok(removed);
            }
            Err(message) => return Err(message),
        };
        if file.remove(host, login) {
            let written = self.write_file(&file);
            drop(guard);
            match written {
                Ok(()) => removed = removed.or(Some(Source::File)),
                Err(message) if removed.is_some() => eprintln!("joy: {message}"),
                Err(message) => return Err(message),
            }
        }
        Ok(removed)
    }

    fn index_add(&self, host: &str, login: Option<&str>) {
        let Some(login) = login else { return };
        let mut logins: Vec<String> = self
            .keys
            .get(INDEX_SERVICE, host)
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        if logins.iter().any(|known| known == login) {
            return;
        }
        logins.push(login.to_string());
        if let Ok(text) = serde_json::to_string(&logins) {
            self.keys.put_checked(INDEX_SERVICE, host, &text);
        }
    }

    fn index_remove(&self, host: &str, login: Option<&str>) {
        let Some(login) = login else { return };
        let Some(text) = self.keys.get(INDEX_SERVICE, host) else {
            return;
        };
        let Ok(mut logins) = serde_json::from_str::<Vec<String>>(&text) else {
            return;
        };
        logins.retain(|known| known != login);
        if logins.is_empty() {
            self.keys.delete(INDEX_SERVICE, host);
        } else if let Ok(text) = serde_json::to_string(&logins) {
            self.keys.put_checked(INDEX_SERVICE, host, &text);
        }
    }

    /// The document as it stands, for a READ. A file that is not there
    /// and a file that cannot be read answer the same thing, because a
    /// read has nothing better to say.
    fn read_file(&self) -> FileVault {
        if self.file.as_os_str().is_empty() {
            return FileVault::default();
        }
        std::fs::read_to_string(&self.file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// The document as it stands, for a READ-MODIFY-WRITE.
    ///
    /// Here the difference matters: rewriting a document that did not
    /// parse would replace every credential in it with the one entry
    /// this call is about. "Not there" is an empty document; "there and
    /// unreadable" is a refusal.
    fn read_file_to_modify(&self) -> Result<FileVault, String> {
        if self.file.as_os_str().is_empty() {
            return Err("this connector call keeps no credentials".to_string());
        }
        let text = match std::fs::read_to_string(&self.file) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(FileVault::default())
            }
            Err(error) => return Err(format!("{}: {error}", self.file.display())),
        };
        if text.trim().is_empty() {
            return Ok(FileVault::default());
        }
        serde_json::from_str(&text).map_err(|error| {
            format!(
                "{} is not a credential file joy can read ({error}); joy will not overwrite it",
                self.file.display()
            )
        })
    }

    /// Hold the document for a read-modify-write.
    ///
    /// The refresh lock of D2.6a is keyed by host and login, and this
    /// file is one document for all of them, so that lock does not
    /// serialise two logins of one host against each other. This one
    /// does, on the same primitive, beside the file it protects.
    ///
    /// Lock order in the whole connector is refresh lock first, then
    /// this one, and never the other way round.
    fn hold_file(&self) -> Result<joy_core::util::file_lock::FileLock, String> {
        if self.file.as_os_str().is_empty() {
            return Err("this connector call keeps no credentials".to_string());
        }
        let mut path = self.file.clone().into_os_string();
        path.push(".lock");
        super::lock::take_at(Path::new(&path))
            .map_err(|busy| format!("{}: {busy}", self.file.display()))
    }

    fn write_file(&self, vault: &FileVault) -> Result<(), String> {
        if self.file.as_os_str().is_empty() {
            return Err("this connector call keeps no credentials".to_string());
        }
        let text = serde_json::to_string_pretty(vault).map_err(|e| e.to_string())?;
        write_private(&self.file, &text)
    }
}

/// The shape of `forge-tokens.json`: hosts, then logins, then records.
/// The empty login key holds the record of a host with no login name,
/// which is the `<host>` form of D2.6's entry addressing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct FileVault {
    #[serde(default)]
    hosts: BTreeMap<String, BTreeMap<String, Record>>,
}

impl FileVault {
    fn get(&self, host: &str, login: Option<&str>) -> Option<Record> {
        let logins = self.hosts.get(host)?;
        match login {
            Some(login) => logins.get(login).cloned(),
            // No login asked for: the unnamed record first, then the
            // only named one there is. Two named logins and no pin is
            // the question D4.1c answers, not this one.
            None => logins
                .get("")
                .cloned()
                .or_else(|| (logins.len() == 1).then(|| logins.values().next().cloned())?),
        }
    }

    fn logins(&self, host: &str) -> Vec<String> {
        self.hosts
            .get(host)
            .map(|logins| {
                logins
                    .keys()
                    .filter(|login| !login.is_empty())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn put(&mut self, host: &str, record: Record) {
        let key = record.login.clone().unwrap_or_default();
        self.hosts
            .entry(host.to_string())
            .or_default()
            .insert(key, record);
    }

    fn remove(&mut self, host: &str, login: Option<&str>) -> bool {
        let Some(logins) = self.hosts.get_mut(host) else {
            return false;
        };
        let key = match login {
            Some(login) => login.to_string(),
            None if logins.contains_key("") => String::new(),
            None if logins.len() == 1 => logins.keys().next().cloned().unwrap_or_default(),
            None => return false,
        };
        let removed = logins.remove(&key).is_some();
        if logins.is_empty() {
            self.hosts.remove(host);
        }
        removed
    }
}

/// The user half of `Entry::new(service, user)`: `<host>` or
/// `<host>|<login>`, exactly as D2.6 writes it.
pub fn user_key(host: &str, login: Option<&str>) -> String {
    match login {
        Some(login) if !login.is_empty() => format!("{host}|{login}"),
        _ => host.to_string(),
    }
}

/// `<config>/forge-tokens.json`, in joy's own configuration directory.
fn default_file() -> PathBuf {
    let dirs = crate::config::joy_config_dirs();
    // The first directory that already holds the file wins, so a person
    // who has one keeps using it; otherwise the most specific candidate
    // is where a new one is written.
    dirs.iter()
        .map(|dir| dir.join(FALLBACK_FILE))
        .find(|path| path.exists())
        .or_else(|| dirs.first().map(|dir| dir.join(FALLBACK_FILE)))
        .unwrap_or_else(|| PathBuf::from(FALLBACK_FILE))
}

/// Write a file only this person can read: 0600 in a 0700 directory
/// (D2.6). On Windows the directory's inherited ACL is what protects
/// it, which is the same protection `%APPDATA%` gives every other
/// credential file on that system.
///
/// The write is a STAGING FILE AND A RENAME, never a truncate and a
/// write. A reader of this file maps "does not parse" to "nothing is
/// stored", so a crash between the truncate and the last byte would
/// make every credential on the machine disappear and every `token`
/// call answer `no-login`. A rename inside one directory replaces the
/// name in one step on every system joy ships to, so no reader ever
/// sees half a document.
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            private_dir(parent);
        }
    }
    let staging = staging_path(path);
    let write = |staging: &Path| -> Result<(), String> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(staging)
            .map_err(|e| format!("{}: {e}", staging.display()))?;
        // An existing staging file keeps its old mode when it is
        // reopened, so the mode is set again rather than trusted.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(staging, std::fs::Permissions::from_mode(0o600));
        }
        file.write_all(text.as_bytes())
            .map_err(|e| format!("{}: {e}", staging.display()))?;
        // The bytes reach the disk before the name does, so a machine
        // that loses power after the rename finds the new document and
        // not an empty one.
        file.sync_all()
            .map_err(|e| format!("{}: {e}", staging.display()))?;
        Ok(())
    };
    if let Err(message) = write(&staging) {
        let _ = std::fs::remove_file(&staging);
        return Err(message);
    }
    if let Err(error) = std::fs::rename(&staging, path) {
        let _ = std::fs::remove_file(&staging);
        return Err(format!("{}: {error}", path.display()));
    }
    Ok(())
}

/// The staging file of one document: the same name with `.new` after
/// it, in the same directory, because a rename is only atomic inside
/// one file system.
fn staging_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".new");
    PathBuf::from(name)
}

#[cfg(unix)]
fn private_dir(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn private_dir(_dir: &Path) {}

#[cfg(test)]
mod tests {

    /// D2.6's "a store that cannot answer" includes one that never
    /// answers: the Secret Service waiting on an unlock prompt nobody
    /// can see (JOY-02AA-ED). The bound turns that silence into `None`
    /// within [`KEYRING_BOUND`], and the second call does not wait at
    /// all, because the silence is remembered for the process.
    #[test]
    fn a_store_that_never_answers_is_a_store_that_could_not_answer() {
        let started = std::time::Instant::now();
        let answer = bounded(|| {
            std::thread::sleep(std::time::Duration::from_secs(3600));
            1
        });
        assert_eq!(answer, None);
        let waited = started.elapsed();
        assert!(
            waited >= KEYRING_BOUND,
            "the bound was not waited for: {waited:?}"
        );
        assert!(
            waited < KEYRING_BOUND * 2,
            "the bound was overshot: {waited:?}"
        );
        let again = std::time::Instant::now();
        assert_eq!(
            bounded(|| 2),
            None,
            "a remembered silence must skip the store"
        );
        assert!(again.elapsed() < std::time::Duration::from_secs(1));
    }
    use super::*;

    fn record(token: &str, login: &str) -> Record {
        Record {
            token: token.to_string(),
            login: Some(login.to_string()),
            scopes: "repo user:email".to_string(),
            ..Record::default()
        }
    }

    /// D2.6's entry addressing, written down once: the service is the
    /// connector's own name and the user is `<host>` or `<host>|<login>`.
    #[test]
    fn an_entry_is_addressed_by_host_and_login_and_nothing_else() {
        assert_eq!(user_key("github.com", None), "github.com");
        assert_eq!(user_key("github.com", Some("")), "github.com");
        assert_eq!(user_key("github.com", Some("scotty")), "github.com|scotty");
        assert_eq!(SERVICE, "joy-forge");
    }

    #[test]
    fn the_file_vault_keeps_one_record_per_host_and_login() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::file_at(dir.path());
        assert!(vault.get("github.com", None).is_none());
        assert_eq!(
            vault.put("github.com", &record("gho_a", "work")).unwrap(),
            Source::File
        );
        vault.put("github.com", &record("gho_b", "scotty")).unwrap();
        let (found, source) = vault.get("github.com", Some("scotty")).unwrap();
        assert_eq!(found.token, "gho_b");
        assert_eq!(source, Source::File);
        let mut logins = vault.logins("github.com");
        logins.sort();
        assert_eq!(logins, vec!["scotty".to_string(), "work".to_string()]);
        // two logins and no name: D4.1c decides that, not the vault
        assert!(vault.get("github.com", None).is_none());
        assert_eq!(
            vault.remove("github.com", Some("work")),
            Ok(Some(Source::File))
        );
        // now there is only one, so an unnamed ask finds it
        assert_eq!(vault.get("github.com", None).unwrap().0.token, "gho_b");
        assert_eq!(vault.remove("github.com", Some("nobody")), Ok(None));
    }

    /// The fallback file is joy's own, and D2.6 says what it must look
    /// like on disk: 0600 in a 0700 directory.
    #[test]
    #[cfg(unix)]
    fn the_fallback_file_is_readable_by_its_owner_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("joy");
        let vault = Vault::file_at(&nested);
        vault
            .put("codeberg.org", &record("cb_token", "scotty"))
            .unwrap();
        let mode = std::fs::metadata(vault.file())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "the token file must be 0600");
        let dir_mode = std::fs::metadata(&nested).unwrap().permissions().mode();
        assert_eq!(dir_mode & 0o777, 0o700, "its directory must be 0700");
        // and it really is joy's own file, not the crate's
        assert!(vault.file().ends_with(FALLBACK_FILE));
    }

    #[test]
    fn a_vault_that_stores_nothing_answers_nothing_and_refuses_to_write() {
        let vault = Vault::none();
        assert!(vault.is_none());
        assert!(vault.get("github.com", None).is_none());
        assert!(vault.logins("github.com").is_empty());
        assert!(vault.put("github.com", &record("gho_a", "work")).is_err());
        assert_eq!(vault.remove("github.com", None), Ok(None));
    }

    #[test]
    fn a_record_knows_when_it_is_past_its_lifetime_against_the_sixty_second_skew() {
        let mut record = record("gho_a", "work");
        assert!(!record.is_expired(), "no lifetime is not an expired one");
        assert!(!record.can_refresh());
        let soon = chrono::Utc::now() + chrono::Duration::seconds(30);
        record.expires_at = Some(soon.to_rfc3339());
        assert!(record.is_expired(), "inside the skew counts as expired");
        let later = chrono::Utc::now() + chrono::Duration::seconds(3600);
        record.expires_at = Some(later.to_rfc3339());
        assert!(!record.is_expired());
        record.refresh_token = Some("r".into());
        record.token_endpoint = Some("https://example.test/token".into());
        assert!(record.can_refresh());
    }

    /// D2.6: the file is ONE document for every host and every login,
    /// and the refresh lock of D2.6a is per host and login, so the
    /// document has a lock of its own. Two writers for two logins of
    /// one host must both survive.
    #[test]
    fn two_logins_of_one_host_written_at_once_both_survive() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::file_at(dir.path());
        let writers: Vec<_> = ["scotty", "work", "ci", "release"]
            .into_iter()
            .map(|login| {
                let vault = vault.clone();
                std::thread::spawn(move || {
                    vault
                        .put("github.com", &record(&format!("gho_{login}"), login))
                        .expect("the entry is written")
                })
            })
            .collect();
        for writer in writers {
            assert_eq!(writer.join().unwrap(), Source::File);
        }
        let mut logins = vault.logins("github.com");
        logins.sort();
        assert_eq!(logins, vec!["ci", "release", "scotty", "work"]);
        for login in ["scotty", "work", "ci", "release"] {
            let (found, _) = vault.get("github.com", Some(login)).unwrap();
            assert_eq!(found.token, format!("gho_{login}"));
        }
    }

    /// The document lock is taken, and not merely documented: a second
    /// writer waits for whoever holds it. flock belongs to the open
    /// file description, so a second handle in this process contends
    /// exactly as a second process does.
    #[test]
    fn a_writer_waits_for_the_document_lock() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::file_at(dir.path());
        vault.put("github.com", &record("gho_a", "work")).unwrap();
        let mut lock_path = vault.file().to_path_buf().into_os_string();
        lock_path.push(".lock");
        let held = super::super::lock::take_at(Path::new(&lock_path)).expect("the document lock");
        let (finished, waited) = std::sync::mpsc::channel();
        let writer = {
            let vault = vault.clone();
            std::thread::spawn(move || {
                let outcome = vault.put("github.com", &record("gho_b", "scotty"));
                let _ = finished.send(());
                outcome
            })
        };
        assert!(
            waited
                .recv_timeout(std::time::Duration::from_millis(300))
                .is_err(),
            "a writer must wait for the document lock, not write beside it"
        );
        drop(held);
        assert_eq!(writer.join().unwrap().unwrap(), Source::File);
        assert_eq!(
            vault.get("github.com", Some("work")).unwrap().0.token,
            "gho_a",
            "the first entry survived the second writer"
        );
        assert_eq!(
            vault.get("github.com", Some("scotty")).unwrap().0.token,
            "gho_b"
        );
    }

    /// D2.6's keychain half, executed: the entry addressing, the login
    /// index that `Entry::new` cannot enumerate, and the removal.
    ///
    /// The store here is an in process one with the semantics of an
    /// operating system's (it persists across `Entry` objects); the
    /// keyring call itself is the one line this cannot cover, and the
    /// case below covers what happens when a store does NOT persist.
    #[test]
    #[cfg(feature = "fake-api")]
    fn the_credential_store_holds_the_entry_the_index_and_nothing_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::fake_keychain_at(dir.path());
        assert_eq!(
            vault.put("github.com", &record("gho_a", "work")).unwrap(),
            Source::Keychain
        );
        vault.put("github.com", &record("gho_b", "scotty")).unwrap();
        let (found, source) = vault.get("github.com", Some("scotty")).unwrap();
        assert_eq!(found.token, "gho_b");
        assert_eq!(source, Source::Keychain);
        assert!(
            !vault.file().exists(),
            "a store that answered means no 0600 file at all"
        );
        // `Entry::new` cannot enumerate, so the logins live in an index
        // entry of their own, and D4.1c's step 3 and its probe order
        // both read it.
        let mut logins = vault.logins("github.com");
        logins.sort();
        assert_eq!(logins, vec!["scotty".to_string(), "work".to_string()]);
        assert_eq!(
            vault.remove("github.com", Some("work")),
            Ok(Some(Source::Keychain))
        );
        assert_eq!(vault.logins("github.com"), vec!["scotty".to_string()]);
        assert!(vault.get("github.com", Some("work")).is_none());
        // the last login goes, and the index goes with it
        assert_eq!(
            vault.remove("github.com", Some("scotty")),
            Ok(Some(Source::Keychain))
        );
        assert!(vault.logins("github.com").is_empty());
        assert_eq!(vault.remove("github.com", Some("scotty")), Ok(None));
    }

    /// The mock detector of D2.6, against the store it was written for.
    ///
    /// keyring's own mock keeps the password inside the `Entry` object
    /// (`CredentialPersistence::EntryOnly`), so the read back through a
    /// NEW entry finds nothing. joy must then write its own 0600 file
    /// instead of believing a store that stored nothing, and must say
    /// `"stored":"file"` truthfully.
    #[test]
    #[cfg(feature = "fake-api")]
    fn a_store_that_persists_nothing_is_detected_and_the_file_takes_over() {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::os_keychain_at(dir.path());
        assert_eq!(
            vault.put("github.com", &record("gho_a", "work")).unwrap(),
            Source::File,
            "a store that cannot answer with what was just written did not persist it"
        );
        let (found, source) = vault.get("github.com", Some("work")).unwrap();
        assert_eq!(found.token, "gho_a");
        assert_eq!(source, Source::File);
        assert!(vault.file().exists(), "the 0600 file of D2.6 took over");
        assert_eq!(vault.logins("github.com"), vec!["work".to_string()]);
        assert_eq!(
            vault.remove("github.com", Some("work")),
            Ok(Some(Source::File))
        );
    }

    /// A document that does not parse is not a document to overwrite: a
    /// reader maps "does not parse" to "nothing is stored", so writing
    /// one entry over it would throw every other credential away.
    #[test]
    fn a_document_joy_cannot_read_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::file_at(dir.path());
        let damaged = "{ this is not what joy wrote";
        std::fs::write(vault.file(), damaged).unwrap();
        assert!(vault.put("github.com", &record("gho_a", "work")).is_err());
        assert!(vault.remove("github.com", Some("work")).is_err());
        assert_eq!(std::fs::read_to_string(vault.file()).unwrap(), damaged);
        // An EMPTY file is not damage: it is a file nothing was written
        // to yet, and the next write fills it.
        std::fs::write(vault.file(), "").unwrap();
        assert_eq!(
            vault.put("github.com", &record("gho_a", "work")).unwrap(),
            Source::File
        );
    }

    /// The write is a staging file and a rename, so no reader ever sees
    /// half a document and no crash can empty the file.
    #[test]
    fn a_write_leaves_no_staging_file_behind_and_replaces_the_document_whole() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::file_at(dir.path());
        vault.put("github.com", &record("gho_a", "work")).unwrap();
        let staging = staging_path(vault.file());
        assert!(!staging.exists(), "the staging file is renamed, not left");
        // Something in the staging file's place is a refusal, not a
        // half written document: the entry that was there stays.
        std::fs::create_dir(&staging).unwrap();
        assert!(vault.put("github.com", &record("gho_b", "scotty")).is_err());
        assert_eq!(
            vault.get("github.com", Some("work")).unwrap().0.token,
            "gho_a"
        );
        assert!(vault.get("github.com", Some("scotty")).is_none());
    }

    /// The record carries the granted set beside the token in the same
    /// entry, which is what D2.7c's local pre check reads.
    #[test]
    fn a_record_round_trips_through_its_json_with_the_granted_set() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::file_at(dir.path());
        let mut stored = record("glpat_a", "scotty");
        stored.scopes = "read_api write_repository".to_string();
        stored.refresh_token = Some("rt".to_string());
        stored.token_endpoint = Some("https://gitlab.com/oauth/token".to_string());
        stored.client_id = Some("cid".to_string());
        vault.put("gitlab.com", &stored).unwrap();
        let (read, _) = vault.get("gitlab.com", Some("scotty")).unwrap();
        assert_eq!(read, stored);
    }
}
