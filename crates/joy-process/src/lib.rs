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
//! A child started WITH the flag owns a console of its own whose window
//! is never shown. To that child the console is a console like any
//! other (it has one, so it lends it on), and its own children inherit
//! it. So a plugin the desktop app starts stays quiet, and so do the
//! `gh` and `curl` calls inside it, without any of them knowing about
//! this crate.
//!
//! Where nobody can see a console, nobody can answer a prompt either.
//! That is a different rule with a different owner: it belongs to the
//! program that asks. This crate only tells the owner - [`headless`] -
//! and joy-core's git boundary turns it into `GIT_TERMINAL_PROMPT=0` and
//! a batch-mode ssh, so a headless fetch fails instead of waiting on a
//! question in a window that does not exist.
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
    let mut command = Command::new(program);
    prepare(&mut command, host_has_console());
    command
}

/// Apply the spawn rules for a host with (or without) a console of its
/// own. Split from [`command`] so the tests can drive both branches.
#[cfg(windows)]
fn prepare(command: &mut Command, host_has_console: bool) {
    use std::os::windows::process::CommandExt;

    /// A console application run without a console window
    /// (`CREATE_NO_WINDOW`, winbase.h).
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    if !host_has_console {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

/// No spawn on unix opens a window, whatever the host is.
#[cfg(not(windows))]
fn prepare(_command: &mut Command, _host_has_console: bool) {}

/// Whether this process has a console it can lend to a child.
///
/// `GetConsoleWindow` answers with the console's window handle, null
/// only when the process has no console at all - a GUI binary. A hidden
/// window is still a console: a child started with `CREATE_NO_WINDOW`
/// has one, and so does a process under Windows Terminal or VS Code,
/// whose pseudo-console window is never shown. Both must lend it on
/// rather than start another, which is why the answer is the null check
/// and not visibility.
///
/// Asked on every spawn, not cached: a host may attach or free a
/// console while it runs (`AttachConsole(ATTACH_PARENT_PROCESS)` at
/// startup is a common idiom), and the query costs nothing next to the
/// process it decides about.
#[cfg(windows)]
fn host_has_console() -> bool {
    use windows_sys::Win32::System::Console::GetConsoleWindow;

    !unsafe { GetConsoleWindow() }.is_null()
}

/// Every unix host lends its stdio on; there is no console to speak of.
#[cfg(not(windows))]
fn host_has_console() -> bool {
    true
}

/// Whether nothing a child shows or asks can reach a person: on Windows
/// a process without a console. Unix answers false, and needs no rule -
/// there a prompt without a controlling terminal already fails on its
/// own (git and ssh both give up without a tty).
///
/// For the programs that ask questions. joy-core's git boundary reads
/// it and disarms git's and ssh's prompts, because an answer nobody can
/// give would otherwise leave the caller waiting for ever.
pub fn headless() -> bool {
    !host_has_console()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_survives_the_preparation() {
        let command = command("git");
        assert_eq!(command.get_program(), "git");
    }

    /// The rule is about the window and nothing else: neither branch
    /// touches the child's environment or arguments. What a program
    /// asks and whether it may is that program's owner's business.
    #[test]
    fn neither_branch_touches_the_environment() {
        for host_has_console in [true, false] {
            let mut command = Command::new("git");
            prepare(&mut command, host_has_console);
            assert_eq!(command.get_envs().count(), 0);
            assert_eq!(command.get_args().count(), 0);
        }
    }

    /// Unix has no console to be without.
    #[cfg(not(windows))]
    #[test]
    fn unix_is_never_headless() {
        assert!(!headless());
    }

    /// The symptom itself: a host without a console must not give its
    /// child a console WINDOW. The child is this test binary again,
    /// re-run with only the probe selected, reporting what it sees.
    ///
    /// Only this branch is worth a spawn. The other one - no flag, the
    /// child inherits whatever the host has - is the operating system's
    /// default, and what the child then sees depends on where the test
    /// runs (a visible conhost window, ConPTY's hidden one, or a fresh
    /// window on a console-less interactive parent); asserting any of
    /// those would test Windows, not this crate.
    #[cfg(windows)]
    #[test]
    fn a_console_less_host_spawns_a_windowless_child() {
        let mut child = Command::new(std::env::current_exe().expect("test binary path"));
        child
            .args([
                "tests::console_probe",
                "--exact",
                "--ignored",
                "--nocapture",
            ])
            .env(PROBE, "1");
        prepare(&mut child, false);
        let output = child.output().expect("run the probe");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let answer = stdout
            .lines()
            .find_map(|line| line.strip_prefix(ANSWER))
            .unwrap_or_else(|| panic!("no probe answer in:\n{stdout}"));
        assert_eq!(answer, "hidden");
    }

    #[cfg(windows)]
    const PROBE: &str = "JOY_PROCESS_CONSOLE_PROBE";
    #[cfg(windows)]
    const ANSWER: &str = "console=";

    /// The child half of the test above, never a test of its own: it is
    /// ignored so a plain `cargo test` does not count it as a pass, and
    /// it only speaks when the parent asked.
    #[cfg(windows)]
    #[test]
    #[ignore = "the child half of a_console_less_host_spawns_a_windowless_child"]
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
