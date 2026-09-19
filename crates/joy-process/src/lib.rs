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
//! program that asks. This crate only tells the owner - [`headless`].
//!
//! Who may be asked anything is NOT this crate's answer any more:
//! `joy_core::host::HostKind`, set once by the entry point of each host,
//! decides that (design D1.1 and D1.10), and `headless` answers false on
//! every unix host so it never could. What is left here is the window
//! rule and nothing else.
//!
//! Async callers build the same command and convert it:
//! `tokio::process::Command::from(joy_process::command("joy-forge"))`.

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
/// For the programs that ask questions: joy's credential helper runner
/// is the one that reads it, and the host kind of D1.1 is what really
/// decides whether a person may be asked. joy runs no git process at
/// all any more (design D3.2), so nothing here is about git.
pub fn headless() -> bool {
    !host_has_console()
}

/// Where `program` resolves on this machine, or None when it is not
/// installed (JOY-0290-EA).
///
/// THE one answer to "is this program here", on all three platforms. The
/// question used to be asked by running `which`, which is a unix program:
/// on Windows that probe failed for every tool, so a machine with Claude
/// Code or Copilot installed was told it had neither.
///
/// Windows needs more than a PATH walk. What a person types is `copilot`,
/// but what an installer put there is `copilot.exe` or, for anything
/// installed through npm, `copilot.cmd` — so the extensions in PATHEXT
/// are what is looked for, and an extensionless file is NOT a candidate
/// there, because Windows cannot execute one. npm does leave such a file
/// beside its `.cmd`: it is the unix shell script, and matching it would
/// resolve to something that cannot run.
///
/// The answer is an ABSOLUTE path, and that is the other half of the fix.
/// Handed a bare name, Rust's own spawn appends `.exe` and nothing else,
/// so it would miss `copilot.cmd` all over again; handed the resolved
/// path it runs the thing, batch shims included.
pub fn resolve(program: impl AsRef<OsStr>) -> Option<std::path::PathBuf> {
    let program = std::path::Path::new(program.as_ref());
    // A name with a path in it is not a PATH lookup; take it as given.
    if program.components().count() > 1 {
        return candidates(program).into_iter().find(|p| is_executable(p));
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .flat_map(|dir| candidates(&dir.join(program)))
        .find(|candidate| is_executable(candidate))
}

/// The file names one program name may wear here, in the order Windows
/// itself prefers. On unix a program is its own name and nothing else.
#[cfg(windows)]
fn candidates(base: &std::path::Path) -> Vec<std::path::PathBuf> {
    let exts: Vec<String> = std::env::var_os("PATHEXT")
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into())
        .to_string_lossy()
        .split(';')
        .filter(|e| !e.is_empty())
        .map(str::to_string)
        .collect();
    // An extension that is already one of PATHEXT's is taken as meant.
    let named = base
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|given| {
            exts.iter()
                .any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(given))
        });
    if named {
        return vec![base.to_path_buf()];
    }
    exts.iter()
        .map(|ext| {
            let mut name = base.as_os_str().to_os_string();
            name.push(ext);
            std::path::PathBuf::from(name)
        })
        .collect()
}

#[cfg(not(windows))]
fn candidates(base: &std::path::Path) -> Vec<std::path::PathBuf> {
    vec![base.to_path_buf()]
}

/// A file somebody could actually start. Unix asks for the execute bit,
/// because a readable file of the right name is not a program; Windows
/// has no such bit, and the extension already carried that meaning.
#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
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

    /// The same question, answered the same way on all three platforms:
    /// a program that IS installed is found, one that is not is not, and
    /// the answer is an absolute path a spawn can use as-is.
    #[test]
    fn a_program_on_the_path_resolves_absolutely() {
        // `cargo` is on the PATH of every machine that runs this test.
        let found = resolve("cargo").expect("cargo is on the PATH of a machine building joy");
        assert!(found.is_absolute(), "{found:?} is not absolute");
        assert!(found.is_file(), "{found:?} is not a file");
        assert_eq!(
            resolve("joy-definitely-not-installed-anywhere"),
            None,
            "a name nobody installed must not resolve"
        );
    }

    /// A name that carries a path is not a PATH lookup; it is checked
    /// where it points, and a directory is not a program.
    #[test]
    fn a_path_is_taken_as_given() {
        let cargo = resolve("cargo").expect("cargo");
        assert_eq!(resolve(&cargo).as_ref(), Some(&cargo));
        assert_eq!(
            resolve(std::env::temp_dir()),
            None,
            "a directory is not a program"
        );
    }

    /// The execute bit is what separates a program from a file that
    /// merely shares its name — the reason a PATH walk over `is_file`
    /// alone was not enough.
    #[cfg(unix)]
    #[test]
    fn unix_wants_the_execute_bit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("joy-resolve-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let plain = dir.join("not-a-program");
        std::fs::write(&plain, "#!/bin/sh\n").expect("write");
        assert_eq!(resolve(&plain), None, "a readable file is not a program");

        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        assert_eq!(resolve(&plain).as_ref(), Some(&plain));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Windows types a name and runs a file: the candidates are the
    /// PATHEXT ones, never the bare name, because an extensionless file
    /// there cannot be executed — and npm leaves exactly such a file
    /// (its unix shell script) beside the `.cmd` that actually runs.
    #[cfg(windows)]
    #[test]
    fn windows_looks_for_the_pathext_names() {
        let names = candidates(std::path::Path::new(r"C:\tools\copilot"));
        assert!(
            names
                .iter()
                .any(|p| p.ends_with("copilot.CMD") || p.ends_with("copilot.cmd")),
            "npm shims must be candidates: {names:?}"
        );
        assert!(
            !names.iter().any(|p| p.extension().is_none()),
            "the bare name cannot run on Windows: {names:?}"
        );
        // an extension PATHEXT already names is taken as meant
        assert_eq!(
            candidates(std::path::Path::new(r"C:\tools\gh.exe")),
            vec![std::path::PathBuf::from(r"C:\tools\gh.exe")]
        );
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
