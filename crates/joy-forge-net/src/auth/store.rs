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

/// One stored credential. Everything the refresh and the answers of
/// D2.4 need is here, so a refresh needs no forge knowledge at all:
/// `token_endpoint` and `client_id` are the ones the login used.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Whether the operating system credential store may be asked.
    keychain: bool,
    /// The 0600 file, used when the store cannot answer.
    file: PathBuf,
}

impl Vault {
    /// The real vault: the credential store first, joy's own file
    /// behind it.
    pub fn real() -> Self {
        Vault {
            keychain: true,
            file: default_file(),
        }
    }

    /// A vault that only ever uses the file under `dir`. This is the
    /// fallback path of D2.6 on purpose, and it is what the tests
    /// drive: a test must never write into the person's own keychain,
    /// and on a machine with a real Secret Service it would.
    pub fn file_at(dir: impl AsRef<Path>) -> Self {
        Vault {
            keychain: false,
            file: dir.as_ref().join(FALLBACK_FILE),
        }
    }

    /// A vault that stores nothing. The pure verbs and every context a
    /// caller builds with [`crate::forge::Ctx::bare`] get this one, so
    /// no library call touches a credential store by accident.
    pub fn none() -> Self {
        Vault {
            keychain: false,
            file: PathBuf::new(),
        }
    }

    /// Whether this vault can hold anything at all.
    pub fn is_none(&self) -> bool {
        !self.keychain && self.file.as_os_str().is_empty()
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
        if self.keychain {
            if let Some(text) = keychain_get(SERVICE, &user_key(host, login)) {
                if let Ok(record) = serde_json::from_str::<Record>(&text) {
                    return Some((record, Source::Keychain));
                }
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
        if self.keychain {
            if let Some(text) = keychain_get(INDEX_SERVICE, host) {
                if let Ok(known) = serde_json::from_str::<Vec<String>>(&text) {
                    logins.extend(known);
                }
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
        let login = record.login.as_deref();
        let text = serde_json::to_string(record).map_err(|e| e.to_string())?;
        if self.keychain && keychain_put(SERVICE, &user_key(host, login), &text) {
            self.index_add(host, login);
            return Ok(Source::Keychain);
        }
        let mut file = self.read_file();
        file.put(host, record.clone());
        self.write_file(&file)?;
        Ok(Source::File)
    }

    /// Remove one record from wherever it is. Answers which store held
    /// it, and `None` when nothing was there.
    pub fn remove(&self, host: &str, login: Option<&str>) -> Option<Source> {
        if self.is_none() {
            return None;
        }
        let mut removed = None;
        if self.keychain && keychain_remove(SERVICE, &user_key(host, login)) {
            self.index_remove(host, login);
            removed = Some(Source::Keychain);
        }
        let mut file = self.read_file();
        if file.remove(host, login) {
            let _ = self.write_file(&file);
            removed = removed.or(Some(Source::File));
        }
        removed
    }

    fn index_add(&self, host: &str, login: Option<&str>) {
        let Some(login) = login else { return };
        let mut logins: Vec<String> = keychain_get(INDEX_SERVICE, host)
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        if logins.iter().any(|known| known == login) {
            return;
        }
        logins.push(login.to_string());
        if let Ok(text) = serde_json::to_string(&logins) {
            keychain_put(INDEX_SERVICE, host, &text);
        }
    }

    fn index_remove(&self, host: &str, login: Option<&str>) {
        let Some(login) = login else { return };
        let Some(text) = keychain_get(INDEX_SERVICE, host) else {
            return;
        };
        let Ok(mut logins) = serde_json::from_str::<Vec<String>>(&text) else {
            return;
        };
        logins.retain(|known| known != login);
        if logins.is_empty() {
            keychain_remove(INDEX_SERVICE, host);
        } else if let Ok(text) = serde_json::to_string(&logins) {
            keychain_put(INDEX_SERVICE, host, &text);
        }
    }

    fn read_file(&self) -> FileVault {
        if self.file.as_os_str().is_empty() {
            return FileVault::default();
        }
        std::fs::read_to_string(&self.file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
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

/// One entry read. Never `new_with_target`, and never a foreign
/// service: this reads what this binary wrote and nothing else.
fn keychain_get(service: &str, user: &str) -> Option<String> {
    let entry = keyring::Entry::new(service, user).ok()?;
    // `NoEntry` is the normal answer for a host nobody signed in to;
    // `NoStorageAccess` and `PlatformFailure` are a store that could not
    // answer. Both end here, and the caller falls through to the 0600
    // file of D2.6.
    entry.get_password().ok()
}

/// One entry written, then read back through a NEW entry.
///
/// The read back is the mock detector of D2.6: keyring's in process
/// mock keeps the password inside the `Entry` object, so a second entry
/// for the same service and user finds nothing, and joy must write its
/// own file instead of believing a store that stored nothing.
fn keychain_put(service: &str, user: &str, secret: &str) -> bool {
    let Ok(entry) = keyring::Entry::new(service, user) else {
        return false;
    };
    if entry.set_password(secret).is_err() {
        return false;
    }
    keychain_get(service, user).as_deref() == Some(secret)
}

fn keychain_remove(service: &str, user: &str) -> bool {
    keyring::Entry::new(service, user)
        .ok()
        .is_some_and(|entry| entry.delete_credential().is_ok())
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
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            private_dir(parent);
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    // An existing file keeps its old mode when it is reopened, so the
    // mode is set again rather than trusted.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
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
        assert_eq!(vault.remove("github.com", Some("work")), Some(Source::File));
        // now there is only one, so an unnamed ask finds it
        assert_eq!(vault.get("github.com", None).unwrap().0.token, "gho_b");
        assert_eq!(vault.remove("github.com", Some("nobody")), None);
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
        assert_eq!(vault.remove("github.com", None), None);
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
