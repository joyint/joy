// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Every child process joy starts is built here (JOY-028F-0B).
//!
//! The reason is Windows and only Windows: a GUI process (the desktop
//! app, built with `windows_subsystem = "windows"`) owns no console, so
//! every console child it spawns is given one - with a WINDOW. The
//! operator's first Windows install showed console windows opening and
//! closing in the background all day long; macOS and Linux never showed
//! it and never can, because a console window is a Windows concept.
//!
//! The rule is one rule: a child gets `CREATE_NO_WINDOW` exactly when
//! THIS process has no console. It is not "always", because a console
//! the CLI inherited is the terminal the person is sitting at: with the
//! flag, the child would get its own invisible console instead and
//! everything that talks to the terminal would break - the pager behind
//! `joy ai tutorial`, the editor, `joy int` and the other plugins with
//! inherited stdio, ssh asking for a passphrase on fetch and push. A GUI
//! host has no console to lend, so there the flag costs nothing.
//!
//! A child started WITH the flag owns a console of its own that has no
//! window, and its own children inherit it. So a plugin the desktop app
//! starts stays quiet, and so do the `gh` and `curl` calls inside it,
//! without any of them knowing about this crate.
//!
//! Where nobody can see a console, nobody can answer a prompt either, so
//! the same branch sets `GIT_TERMINAL_PROMPT=0`: an app-side `git fetch`
//! that wants credentials has to fail visibly instead of hanging on a
//! question in a window that does not exist. Git Credential Manager is
//! unaffected, it draws its own window.
//!
//! Async callers build the same command and convert it:
//! `tokio::process::Command::from(joy_process::command("git"))`.

// The one crate that may build a Command directly; `clippy.toml` sends
// everyone else here.
#![allow(clippy::disallowed_methods)]

use std::ffi::OsStr;
use std::process::Command;

/// A [`Command`] for `program`, prepared for this host.
///
/// The only entry point: `clippy.toml` disallows
/// `std::process::Command::new` and `tokio::process::Command::new`
/// everywhere else, so a new spawn cannot quietly skip the rule.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    // Nothing mutates it off Windows, because nothing has to.
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut command = Command::new(program);
    #[cfg(windows)]
    prepare(&mut command, host_has_console());
    command
}

/// Apply the spawn rules for a host with (or without) a console of its
/// own. Split from [`command`] so the tests can drive both branches on
/// one machine; on unix there are no rules at all, because no spawn
/// there opens a window, whatever the host is.
#[cfg(windows)]
fn prepare(command: &mut Command, host_has_console: bool) {
    use std::os::windows::process::CommandExt;

    /// A console application run without a console window
    /// (`CREATE_NO_WINDOW`, winbase.h).
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    if !host_has_console {
        command.creation_flags(CREATE_NO_WINDOW);
        command.env("GIT_TERMINAL_PROMPT", "0");
    }
}

/// Whether this process has a console it can lend to a child.
///
/// `GetConsoleWindow` answers with the console's window handle, null
/// when the process has no console at all (a GUI binary) - and also for
/// a console without a window, which is what a process started with
/// `CREATE_NO_WINDOW` has. Both answers are right for us: the first
/// needs the flag, the second may as well pass it on.
///
/// The console cannot appear or disappear under a running process
/// except through `AllocConsole`/`FreeConsole`, which joy never calls
/// outside its own tests, so the answer is looked up once.
#[cfg(windows)]
fn host_has_console() -> bool {
    use std::sync::OnceLock;
    use windows_sys::Win32::System::Console::GetConsoleWindow;

    static HAS_CONSOLE: OnceLock<bool> = OnceLock::new();
    *HAS_CONSOLE.get_or_init(|| !unsafe { GetConsoleWindow() }.is_null())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_survives_the_preparation() {
        let command = command("git");
        assert_eq!(command.get_program(), "git");
    }

    /// Unix hosts open no windows and answer no prompts differently
    /// from what the caller set up, so nothing is added here.
    #[cfg(not(windows))]
    #[test]
    fn unix_commands_carry_no_extra_environment() {
        let command = command("git");
        assert_eq!(command.get_envs().count(), 0);
    }

    /// A host with a console lends it out, and a child on the person's
    /// terminal can still be asked for a passphrase.
    #[cfg(windows)]
    #[test]
    fn a_console_host_leaves_the_child_on_the_terminal() {
        let mut command = Command::new("git");
        prepare(&mut command, true);
        assert!(
            !command
                .get_envs()
                .any(|(key, _)| key == "GIT_TERMINAL_PROMPT"),
            "a child that inherits the terminal can still be asked for credentials"
        );
    }

    /// The symptom itself: a host without a console must not give its
    /// child a console WINDOW. The child is this test binary again,
    /// re-run with one test selected, reporting what it sees.
    #[cfg(windows)]
    #[test]
    fn a_console_less_host_spawns_a_windowless_child() {
        let mut child = probe_command();
        prepare(&mut child, false);
        assert_eq!(probe_answer(child), "hidden");
    }

    /// The other branch, on the same machine: a child that inherits a
    /// visible console reports it. Skipped where the test binary itself
    /// has no console (a CI shell can run without one) - then there is
    /// nothing to inherit and nothing to assert.
    #[cfg(windows)]
    #[test]
    fn a_console_host_hands_its_console_down() {
        if !host_has_console() {
            return;
        }
        let mut child = probe_command();
        prepare(&mut child, true);
        assert_eq!(probe_answer(child), "visible");
    }

    /// This binary, re-invoked with only [`console_probe`] selected.
    #[cfg(windows)]
    fn probe_command() -> Command {
        let mut command = Command::new(std::env::current_exe().expect("test binary path"));
        command
            .args(["tests::console_probe", "--exact", "--nocapture"])
            .env(PROBE, "1");
        command
    }

    #[cfg(windows)]
    fn probe_answer(mut command: Command) -> String {
        let output = command.output().expect("run the probe");
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout
            .lines()
            .find_map(|line| line.strip_prefix(ANSWER))
            .unwrap_or_else(|| panic!("no probe answer in:\n{stdout}"))
            .to_string()
    }

    #[cfg(windows)]
    const PROBE: &str = "JOY_PROCESS_CONSOLE_PROBE";
    #[cfg(windows)]
    const ANSWER: &str = "console=";

    /// Not a test of its own: the child half of the two above. It only
    /// speaks when [`probe_command`] asked for it, and a plain
    /// `cargo test` run walks past it.
    #[cfg(windows)]
    #[test]
    fn console_probe() {
        use windows_sys::Win32::System::Console::GetConsoleWindow;
        use windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible;

        if std::env::var_os(PROBE).is_none() {
            return;
        }
        let window = unsafe { GetConsoleWindow() };
        let visible = !window.is_null() && unsafe { IsWindowVisible(window) } != 0;
        println!("{ANSWER}{}", if visible { "visible" } else { "hidden" });
    }
}
