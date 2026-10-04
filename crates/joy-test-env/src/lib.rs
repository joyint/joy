// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! What a joy test never inherits from the person's shell and session,
//! said once (JOY-02BB-C7).
//!
//! A developer's shell carries joy's own state: `JOY_SESSION` of the
//! project they work in, sometimes `JOY_USER` or `JOY_PASSPHRASE`, and
//! git's identity variables. A test that starts `joy`, or calls joy-core
//! in its own process, inherits all of it unless it says otherwise, and
//! then answers for the developer instead of for the person the case
//! built. Every test binary used to keep its own list of what to remove,
//! and the lists drifted: with those three variables set, 47 cases in 10
//! binaries failed on a machine where CI was green.
//!
//! The rule is a prefix, not a list, so a variable joy learns to read
//! tomorrow is covered the day it is added: a test inherits no variable
//! whose name starts with `JOY_`, and none of [`GIT_IDENTITY`]. What a
//! case wants set, it sets itself, after this.
//!
//! The person's session bus is the third thing a test must not reach.
//! The forge connector keeps tokens in the operating system's credential
//! store, on Linux the Secret Service on the D-Bus session bus, and a
//! test that starts the connector with the developer's bus address reads
//! the developer's own keychain. On 2026-10-04 gnome-keyring aborted
//! during a run of this suite, on a client that was gone in the middle of
//! its negotiation; a watch on the bus then showed the suite asking the
//! Secret Service for its test hosts. So every test gets
//! [`NO_SESSION_BUS`]: the machine the
//! 0600 file fallback was written for, a host whose credential store
//! cannot be reached. This is Linux only. The macOS keychain and the
//! Windows credential manager are not behind a variable, so there a test
//! stays away from the store by building its own vault, as before.
//!
//! The fourth is the person's forge CLI. joy asks `gh`, `glab` and `tea`
//! for the token they hold, so that nobody signs in twice, and a test
//! that runs with the developer's PATH asks the developer's own `gh` and
//! is handed their real token. So a test finds stand-ins first on PATH
//! ([`forge_cli_dir`]): the three programs, installed and signed in
//! nowhere, the same on every machine. A case that needs a `gh` that
//! answers puts its own before them, as `forge_sign_in.rs` does. The
//! variables those CLIs and joy take a token or a config directory from
//! ([`FORGE_CLI`]) are not inherited either. Unix only: Windows starts a
//! program by its `.exe`, which a script cannot stand in for.
//!
//! What this crate does NOT take away is the person's home directory. A
//! case that reads `~/.config/gh/hosts.yml` or `~/.gitconfig` through
//! joy has to bring its own HOME, as the cases about identity do.
//!
//! Two entries, one for each way a test reaches joy:
//!
//! - [`isolate!`] once in the root of every test binary: each file
//!   directly under a `tests` directory, and under `#[cfg(test)]` the
//!   root of every crate (`lib.rs`, `main.rs`, `src/bin/*.rs`). It sweeps
//!   the process when the binary loads, before the first case runs, so
//!   everything a case reaches in its own process, and everything
//!   joy-core starts from there, runs without the person's session.
//! - [`command`] for every process a test starts itself, `joy` and
//!   anything that may start it (a hook, a shell).
//!
//! Which case reaches the outside cannot be read off its source
//! (`sign_in_against_fakes` sets no variable and still had the person's
//! `gh` ask their keychain; a unit test in joy-forge-net's `src` ran
//! their `glab`), which is why the rule is per binary and leaves out
//! only what lies below it: this crate, and joy-process, which this
//! crate is built on and whose cases start nothing but their own test
//! binary. `just guard-test-env` holds both halves.

use std::ffi::{OsStr, OsString};
use std::process::Command;

/// The prefix of every variable joy reads for itself.
pub const JOY_PREFIX: &str = "JOY_";

/// The variables git, and joy with it, take a person's address from
/// when no config names one.
pub const GIT_IDENTITY: [&str; 3] = ["GIT_AUTHOR_EMAIL", "GIT_COMMITTER_EMAIL", "EMAIL"];

/// The variables a forge CLI, and joy with it, takes a token or its
/// config directory from.
pub const FORGE_CLI: [&str; 9] = [
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
    "GITLAB_TOKEN",
    "GITEA_TOKEN",
    "GH_CONFIG_DIR",
    "GLAB_CONFIG_DIR",
    "TEA_CONFIG_DIR",
];

/// The directory of the forge CLI stand-ins, written by this crate's
/// build script: `gh`, `glab` and `tea`, installed and signed in nowhere.
pub fn forge_cli_dir() -> &'static std::path::Path {
    std::path::Path::new(env!("JOY_TEST_ENV_FORGE_CLIS"))
}

/// PATH with the forge CLI stand-ins before everything the person has.
fn path_without_the_persons_forge_clis() -> OsString {
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let stand_ins = forge_cli_dir().to_path_buf();
    let rest = std::env::split_paths(&inherited).filter(|dir| *dir != stand_ins);
    std::env::join_paths(std::iter::once(stand_ins.clone()).chain(rest)).unwrap_or(inherited)
}

/// The variable that names the D-Bus session bus.
pub const SESSION_BUS: &str = "DBUS_SESSION_BUS_ADDRESS";

/// A session bus address nothing answers at: the Secret Service is out
/// of reach, at once and without a wait.
pub const NO_SESSION_BUS: &str = "unix:path=/nonexistent/joy-test-no-bus";

/// Is `name` a variable a test must not inherit?
///
/// Windows reads a variable whatever the case of its name, so there
/// `Joy_Session` is `JOY_SESSION` and is compared that way.
pub fn is_inherited_state(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let name = if cfg!(windows) {
        name.to_ascii_uppercase()
    } else {
        name.to_string()
    };
    name.starts_with(JOY_PREFIX)
        || GIT_IDENTITY.contains(&name.as_str())
        || FORGE_CLI.contains(&name.as_str())
}

/// The variables of this process, right now, that a test must not
/// inherit.
pub fn inherited_state() -> Vec<OsString> {
    std::env::vars_os()
        .map(|(name, _)| name)
        .filter(|name| is_inherited_state(name))
        .collect()
}

/// A [`Command`] for `program` that inherits none of the person's joy
/// state and cannot reach their session bus: [`joy_process::command`],
/// with every variable of [`inherited_state`] removed and
/// [`NO_SESSION_BUS`] as the bus. A case that wants one of them sets it
/// on the returned command.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = joy_process::command(program);
    for name in inherited_state() {
        command.env_remove(name);
    }
    command.env(SESSION_BUS, NO_SESSION_BUS);
    command.env("PATH", path_without_the_persons_forge_clis());
    command
}

/// Sweep THIS process: remove the person's joy state, take the session
/// bus away and put the forge CLI stand-ins first on PATH.
fn sweep() {
    for name in inherited_state() {
        std::env::remove_var(name);
    }
    std::env::set_var(SESSION_BUS, NO_SESSION_BUS);
    std::env::set_var("PATH", path_without_the_persons_forge_clis());
}

/// [`sweep`], once per process. This is what [`isolate!`] runs when the
/// test binary loads; nothing else has to call it.
///
/// Once, because the environment is process wide and the cases of a
/// binary run side by side: a second sweep would take away what a case
/// has set for itself in the meantime.
pub fn scrub_process() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(sweep);
}

/// Leave the person's shell and session when this test binary LOADS,
/// before its first case runs: [`scrub_process`], from a function the
/// loader calls ahead of `main`.
///
/// Once in the root of every test binary, see the crate documentation.
/// Rust has no hook that runs before the tests of a binary, and a call
/// at the start of every case is a line somebody forgets, in the one
/// case that then reaches the developer's keychain. The loader has such
/// a hook on every system joy builds for, and this is its plain form: a
/// function pointer in the section the loader runs through before
/// `main` (`.init_array` on ELF, `__mod_init_func` on Mach-O,
/// `.CRT$XCU` on Windows). At that point the process has one thread, so
/// moving the environment there races with nothing.
#[macro_export]
macro_rules! isolate {
    () => {
        #[used]
        #[cfg_attr(all(unix, not(target_vendor = "apple")), link_section = ".init_array")]
        #[cfg_attr(target_vendor = "apple", link_section = "__DATA,__mod_init_func")]
        #[cfg_attr(windows, link_section = ".CRT$XCU")]
        static JOY_TEST_ENV_ISOLATE: extern "C" fn() = {
            extern "C" fn isolate() {
                $crate::scrub_process();
            }
            isolate
        };
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_names_joys_own_variables_and_the_git_identity() {
        for name in [
            "JOY_SESSION",
            "JOY_USER",
            "JOY_PASSPHRASE",
            "JOY_A_VARIABLE_OF_TOMORROW",
            "GIT_AUTHOR_EMAIL",
            "GIT_COMMITTER_EMAIL",
            "EMAIL",
            "GH_TOKEN",
            "GITLAB_TOKEN",
            "GH_CONFIG_DIR",
        ] {
            assert!(is_inherited_state(OsStr::new(name)), "{name}");
        }
        for name in ["HOME", "PATH", "GIT_CONFIG_NOSYSTEM", "ENJOY_THIS"] {
            assert!(!is_inherited_state(OsStr::new(name)), "{name}");
        }
        // The case of a name counts where the system says it does.
        assert_eq!(is_inherited_state(OsStr::new("joy_session")), cfg!(windows));
    }

    /// The stand-ins are programs, and they know nobody.
    #[cfg(unix)]
    #[test]
    fn the_forge_cli_stand_ins_are_installed_and_signed_in_nowhere() {
        for name in ["gh", "glab", "tea"] {
            let status = joy_process::command(forge_cli_dir().join(name))
                .args(["auth", "token"])
                .status()
                .unwrap_or_else(|e| panic!("{name} runs: {e}"));
            assert!(!status.success(), "{name} hands out no token");
        }
    }

    /// ONE case for everything that moves the environment of this
    /// process, which is process wide: a second case beside it would
    /// read what this one set.
    #[test]
    fn a_command_and_the_process_lose_what_the_shell_carried() {
        std::env::set_var("JOY_SESSION", "joy_s_of-the-developer");
        std::env::set_var("JOY_USER", "developer@example.com");
        std::env::set_var("GIT_AUTHOR_EMAIL", "developer@example.com");

        let mut command = command("joy");
        let removed = |command: &Command, name: &str| {
            command
                .get_envs()
                .any(|(key, value)| key == OsStr::new(name) && value.is_none())
        };
        for name in ["JOY_SESSION", "JOY_USER", "GIT_AUTHOR_EMAIL"] {
            assert!(removed(&command, name), "{name} is removed");
        }

        assert!(
            command.get_envs().any(|(key, value)| {
                key == OsStr::new(SESSION_BUS) && value == Some(OsStr::new(NO_SESSION_BUS))
            }),
            "the command cannot reach the person's session bus"
        );

        let path = command
            .get_envs()
            .find_map(|(key, value)| (key == OsStr::new("PATH")).then_some(value))
            .flatten()
            .expect("the command has a PATH of its own");
        assert_eq!(
            std::env::split_paths(path).next().as_deref(),
            Some(forge_cli_dir()),
            "the forge CLI stand-ins come before the person's"
        );

        // What the case sets after that stands.
        command.env("JOY_SESSION", "joy_s_of-the-case");
        assert!(command.get_envs().any(|(key, value)| {
            key == OsStr::new("JOY_SESSION") && value == Some(OsStr::new("joy_s_of-the-case"))
        }));

        sweep();
        for name in ["JOY_SESSION", "JOY_USER", "GIT_AUTHOR_EMAIL"] {
            assert!(std::env::var_os(name).is_none(), "{name} left the process");
        }
        assert!(inherited_state().is_empty());
        assert_eq!(
            std::env::var_os(SESSION_BUS).as_deref(),
            Some(OsStr::new(NO_SESSION_BUS)),
            "and the process cannot reach the session bus"
        );
    }
}
