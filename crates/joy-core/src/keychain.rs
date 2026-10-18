// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The one door to the operating system's credential store
//! (JOY-02DB-25): the macOS keychain, the Windows credential manager,
//! on Linux the Secret Service with the kernel keyutils store as its
//! cache.
//!
//! Nothing else in joy, the connector or the app talks to the keyring
//! crate. Three callers used to, each with its own `keyring::Entry`:
//! the connector's token store (`joy-forge-net`, `auth::store`), and in
//! the desktop app the remembered seed (`crypt_ops`) and the platform
//! claim and session bearer (`platform_ops`). Each of them already
//! knows what to do on a machine without a store, and that is what the
//! door gives them when it is closed.
//!
//! An entry is addressed by service and user and nothing else, through
//! `keyring::Entry::new` and never `new_with_target`, and every call
//! builds a NEW entry: what this door answers is what the store holds,
//! never what one entry object happens to remember.
//!
//! **Closed for tests.** [`TEST_SWITCH`] set to `1` closes the door: every
//! call then answers [`Error::Unavailable`] without reaching the store,
//! which is exactly what a machine without a credential store answers,
//! so every caller takes the fallback it already has (the connector its
//! 0600 file, the app its "cannot remember on this device"). It exists
//! for tests alone and `joy_test_env::isolate!` sets it in every test
//! binary, so every process a test starts inherits it, on every system.
//! A person never sets it, and there is no config setting beside it: the
//! connector runs without a project.
//!
//! What a store that does not answer at all costs (a Secret Service
//! waiting on an unlock prompt nobody can see) is the caller's to bound,
//! because the caller knows how long it may wait: the connector bounds
//! every call (`joy-forge-net`, `auth::store`, `KEYRING_BOUND`).
//!
//! Behind the `keychain` feature, so a build that keeps no secret of a
//! person's (the platform) carries no keyring at all. Which store
//! backends that feature asks for, and why `keychain-vendored` exists
//! beside it, is said in this crate's Cargo.toml.

/// The variable that closes the door for a test run, set to `1`. For
/// tests only, see the module documentation.
pub const TEST_SWITCH: &str = "JOY_TEST_NO_KEYCHAIN";

/// Why the store gave no secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The store answered, and it holds nothing under this service and
    /// user: the normal answer for something never stored.
    NoEntry,
    /// The store could not answer: there is none on this machine, it is
    /// locked or refused, the entry could not be addressed, or the door
    /// is closed for a test run ([`TEST_SWITCH`]). The text is the
    /// store's own.
    Unavailable(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoEntry => write!(f, "No matching entry found in secure storage"),
            Error::Unavailable(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<keyring::Error> for Error {
    fn from(error: keyring::Error) -> Self {
        match error {
            keyring::Error::NoEntry => Error::NoEntry,
            other => Error::Unavailable(other.to_string()),
        }
    }
}

/// The secret stored under `service` and `user`.
pub fn get(service: &str, user: &str) -> Result<String, Error> {
    through(is_closed(), service, user, |entry| entry.get_password())
}

/// Store `secret` under `service` and `user`, replacing what was there.
pub fn set(service: &str, user: &str, secret: &str) -> Result<(), Error> {
    through(is_closed(), service, user, |entry| {
        entry.set_password(secret)
    })
}

/// Remove what is stored under `service` and `user`. [`Error::NoEntry`]
/// when nothing was.
pub fn delete(service: &str, user: &str) -> Result<(), Error> {
    through(is_closed(), service, user, |entry| {
        entry.delete_credential()
    })
}

/// Whether [`TEST_SWITCH`] closes the door. Read at every call, so it
/// holds from the moment the variable is set.
fn is_closed() -> bool {
    std::env::var_os(TEST_SWITCH).is_some_and(|value| value == "1")
}

/// One call through a new entry, unless the door is `closed`.
fn through<T>(
    closed: bool,
    service: &str,
    user: &str,
    call: impl FnOnce(&keyring::Entry) -> keyring::Result<T>,
) -> Result<T, Error> {
    if closed {
        return Err(Error::Unavailable(format!(
            "the credential store is closed for this test run ({TEST_SWITCH})"
        )));
    }
    let entry = keyring::Entry::new(service, user)?;
    Ok(call(&entry)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// isolate! closes the door for every test binary, with the name
    /// this module reads.
    #[test]
    fn the_test_binary_finds_the_door_closed() {
        assert_eq!(TEST_SWITCH, joy_test_env::NO_KEYCHAIN);
        assert!(is_closed(), "isolate! set {TEST_SWITCH}");
        let closed = |answer: Result<(), Error>| matches!(answer, Err(Error::Unavailable(ref why)) if why.contains(TEST_SWITCH));
        assert!(closed(get("joy-core-test", "probe").map(|_| ())));
        assert!(closed(set("joy-core-test", "probe", "secret")));
        assert!(closed(delete("joy-core-test", "probe")));
    }

    /// An open door reaches the store and hands its answers on. The
    /// store here is keyring's in memory mock, which keeps a secret
    /// inside the one entry it was written through: a write succeeds,
    /// and a read or a removal through a new entry finds nothing.
    #[test]
    fn an_open_door_asks_the_store_through_a_new_entry_every_time() {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        assert_eq!(
            through(false, "joy-core-test", "probe", |entry| entry
                .set_password("secret")),
            Ok(())
        );
        assert_eq!(
            through(false, "joy-core-test", "probe", |entry| entry
                .get_password()),
            Err(Error::NoEntry)
        );
        assert_eq!(
            through(false, "joy-core-test", "probe", |entry| entry
                .delete_credential()),
            Err(Error::NoEntry)
        );
        assert!(matches!(
            Error::from(keyring::Error::NoStorageAccess("locked".into())),
            Error::Unavailable(why) if why.contains("locked")
        ));
    }
}
