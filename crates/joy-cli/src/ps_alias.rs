// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Windows PowerShell alias hint (JOY-01C3-90).
//!
//! On Windows, the bare command `joy` resolves to
//! `C:\Windows\System32\joy.cpl` (Game Controllers) before the installed
//! `joy.exe`. A `Set-Alias joy joy.exe` line in the user's PowerShell
//! profile fixes interactive PowerShell sessions. This module detects
//! whether that alias is present and drives the hint surfaces (help tip,
//! first-run prompt, `joy update` prompt).
//!
//! The pure logic (alias regex, reading PowerShell's answer, append payload,
//! edition detection from a process name) is cross-platform and unit-tested on
//! every OS. Only the OS glue (parent-process inspection, asking PowerShell for
//! its profile path, file IO) is `#[cfg(windows)]`; the public surface is a
//! no-op off Windows.

// The pure helpers below are used by the `#[cfg(windows)]` glue and the unit
// tests. On a non-Windows, non-test build they are unused; that is an artifact
// of the platform gating, not real dead code.
#![cfg_attr(not(windows), allow(dead_code))]

/// The alias line we offer to add.
const ALIAS_LINE: &str = "Set-Alias joy joy.exe";

/// PowerShell edition, used to choose the right `$PROFILE` path.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PsEdition {
    /// PowerShell 7+ (`pwsh.exe`): `Documents\PowerShell\...`.
    Core,
    /// Windows PowerShell 5.1 (`powershell.exe`): `Documents\WindowsPowerShell\...`.
    WindowsPowerShell,
}

/// Map a parent-process executable name to a PowerShell edition. `None` for
/// anything that is not PowerShell (`cmd.exe`, a CI runner, unknown), which is
/// the signal to emit nothing at all.
fn ps_edition_from_exe(exe_name: &str) -> Option<PsEdition> {
    let lower = exe_name.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
    match stem {
        "pwsh" => Some(PsEdition::Core),
        "powershell" => Some(PsEdition::WindowsPowerShell),
        _ => None,
    }
}

/// The profile path out of PowerShell's own answer (one path on stdout).
/// `None` when it answered nothing usable.
fn profile_path_from_output(stdout: &str) -> Option<std::path::PathBuf> {
    let line = stdout.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(std::path::PathBuf::from(line))
}

/// Whether the profile already defines a `joy` alias. Loose by design: any
/// target value counts (`joy.exe`, an absolute path, a relative path). The user
/// owns the line; if it exists we leave it alone.
fn alias_present(profile_contents: &str) -> bool {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"(?im)^\s*Set-Alias\s+(-Name\s+)?joy\s+")
            .expect("static alias regex is valid")
    });
    re.is_match(profile_contents)
}

/// The exact bytes to append so the alias line lands on its own, with a single
/// blank line separating it from existing content (per the spec). Empty file:
/// just the line. Already ends in a blank line: just the line. Otherwise: enough
/// newlines to leave one blank line before it.
fn append_payload(existing: &str) -> String {
    if existing.is_empty() || existing.ends_with("\n\n") {
        format!("{ALIAS_LINE}\n")
    } else if existing.ends_with('\n') {
        format!("\n{ALIAS_LINE}\n")
    } else {
        format!("\n\n{ALIAS_LINE}\n")
    }
}

// ---------------------------------------------------------------------------
// Windows glue + public surface. Off Windows the surface compiles to no-ops so
// call sites stay free of `#[cfg]`.
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod platform {
    use super::*;
    use std::path::{Path, PathBuf};

    /// The PowerShell edition of the parent process, or `None` when the parent
    /// is not PowerShell (cmd, a CI runner, unknown), in which case we emit
    /// nothing anywhere.
    fn parent_ps_edition() -> Option<PsEdition> {
        use sysinfo::{Pid, ProcessesToUpdate, System};
        let mut sys = System::new();
        sys.refresh_processes(ProcessesToUpdate::All, true);
        let me = Pid::from_u32(std::process::id());
        let parent_pid = sys.process(me)?.parent()?;
        let parent = sys.process(parent_pid)?;
        ps_edition_from_exe(&parent.name().to_string_lossy())
    }

    /// `(profile path for the calling shell, whether the alias is already
    /// present, the PowerShell edition)`. `None` when the calling shell is not
    /// PowerShell or the user profile directory is unknown.
    fn target_profile() -> Option<(PathBuf, bool, PsEdition)> {
        let edition = parent_ps_edition()?;
        let path = ask_profile_path(edition)?;
        let present = std::fs::read_to_string(&path)
            .map(|c| alias_present(&c))
            .unwrap_or(false);
        Some((path, present, edition))
    }

    /// Where PowerShell keeps the profile that runs in EVERY host: the console,
    /// Windows Terminal, and the VS Code integrated console, which loads its own
    /// `Microsoft.VSCode_profile.ps1` and would never see a host-specific one.
    ///
    /// Asked, never built. `Documents` moves (OneDrive Known Folder Move,
    /// corporate folder redirection), so a path assembled from `%USERPROFILE%`
    /// points at a file PowerShell never loads: the alias then exists, looks
    /// right, and does nothing.
    fn ask_profile_path(edition: PsEdition) -> Option<PathBuf> {
        let out = std::process::Command::new(ps_exe(edition))
            .args(["-NoProfile", "-Command", "$PROFILE.CurrentUserAllHosts"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        profile_path_from_output(&String::from_utf8_lossy(&out.stdout))
    }

    /// The PowerShell executable name for an edition.
    fn ps_exe(edition: PsEdition) -> &'static str {
        match edition {
            PsEdition::Core => "pwsh",
            PsEdition::WindowsPowerShell => "powershell",
        }
    }

    /// Whether the effective execution policy blocks an unsigned local script
    /// like `$PROFILE` (so the alias would never load). Reads it via the calling
    /// edition's own `Get-ExecutionPolicy`; on any error we assume not blocked so
    /// as not to nag.
    fn scripts_disabled(edition: PsEdition) -> bool {
        let out = std::process::Command::new(ps_exe(edition))
            .args(["-NoProfile", "-Command", "Get-ExecutionPolicy"])
            .output();
        match out {
            Ok(o) => {
                let policy = String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .to_ascii_lowercase();
                policy == "restricted" || policy == "allsigned"
            }
            Err(_) => false,
        }
    }

    /// Run `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned`. Returns whether
    /// it succeeded plus any error text (e.g. when a group policy locks it).
    fn enable_local_scripts(edition: PsEdition) -> (bool, String) {
        let out = std::process::Command::new(ps_exe(edition))
            .args([
                "-NoProfile",
                "-Command",
                "Set-ExecutionPolicy -Scope CurrentUser RemoteSigned -Force",
            ])
            .output();
        match out {
            Ok(o) if o.status.success() => (true, String::new()),
            Ok(o) => (false, String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => (false, e.to_string()),
        }
    }

    /// Append the alias line to the profile, creating the file and its parent
    /// directory if needed. Written as plain UTF-8 without a BOM: the line is
    /// pure ASCII, which every PowerShell edition reads correctly, and a BOM
    /// would otherwise sit in front of the alias and defeat our own detection.
    fn write_alias(path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let existing = std::fs::read_to_string(path).unwrap_or_default();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        file.write_all(append_payload(&existing).as_bytes())
    }

    /// Passive help tip: printed at the end of `joy help` / `joy -h` when the
    /// alias is missing. No prompt.
    pub fn print_help_tip() {
        if let Some((_, present, _)) = target_profile() {
            if !present {
                println!();
                println!(
                    "Tip: 'joy' alone opens Windows Game Controllers. To make 'joy' launch this tool:"
                );
                println!("  Add-Content $PROFILE.CurrentUserAllHosts \"{ALIAS_LINE}\"");
            }
        }
    }

    /// Interactive offer used by the first-run and `joy update` surfaces: when
    /// the alias is missing and the session is interactive, explain the issue
    /// and offer to add the line.
    pub fn offer_alias_fix() {
        if !crate::prompt::is_interactive() {
            return;
        }
        let Some((path, present, edition)) = target_profile() else {
            return;
        };
        if present {
            return;
        }
        println!();
        println!("Tip: 'joy' alone opens Windows Game Controllers. Add `{ALIAS_LINE}`");
        println!("to your PowerShell profile so 'joy' launches this tool.");
        if !matches!(crate::prompt::ask_yn("Do it now?", true), Ok(true)) {
            return;
        }

        // The alias lives in the profile, which only runs if the execution policy
        // allows local scripts. On a default-Restricted box it would never load,
        // so offer to allow local scripts first, only when actually needed, and
        // only on explicit consent (it changes a Windows security setting).
        if scripts_disabled(edition) {
            println!();
            println!(
                "PowerShell currently blocks scripts, so the profile (and the alias) will not load."
            );
            if matches!(
                crate::prompt::ask_yn(
                    "Allow local scripts (Set-ExecutionPolicy -Scope CurrentUser RemoteSigned) now?",
                    false,
                ),
                Ok(true)
            ) {
                match enable_local_scripts(edition) {
                    (true, _) => println!("Execution policy set to RemoteSigned (current user)."),
                    (false, err) => {
                        println!(
                            "Could not change the execution policy (it may be locked by your \
                             administrator). Set it manually:"
                        );
                        println!("  Set-ExecutionPolicy -Scope CurrentUser RemoteSigned");
                        if !err.is_empty() {
                            println!("  ({err})");
                        }
                    }
                }
            } else {
                println!(
                    "Skipped. The alias will not load until you run: \
                     Set-ExecutionPolicy -Scope CurrentUser RemoteSigned"
                );
            }
        }

        match write_alias(&path) {
            Ok(()) => println!("Done. Restart your shell."),
            Err(e) => eprintln!("Could not update {}: {e}", path.display()),
        }
    }
}

#[cfg(windows)]
pub use platform::{offer_alias_fix, print_help_tip};

/// No-op off Windows: the alias hint is a Windows-only concern.
#[cfg(not(windows))]
pub fn print_help_tip() {}

/// No-op off Windows: the alias hint is a Windows-only concern.
#[cfg(not(windows))]
pub fn offer_alias_fix() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edition_from_exe_name() {
        assert_eq!(ps_edition_from_exe("pwsh.exe"), Some(PsEdition::Core));
        assert_eq!(ps_edition_from_exe("pwsh"), Some(PsEdition::Core));
        assert_eq!(
            ps_edition_from_exe("PowerShell.EXE"),
            Some(PsEdition::WindowsPowerShell)
        );
        assert_eq!(ps_edition_from_exe("cmd.exe"), None);
        assert_eq!(ps_edition_from_exe("bash"), None);
        assert_eq!(ps_edition_from_exe("python.exe"), None);
    }

    #[test]
    fn the_profile_path_comes_from_powershells_own_answer() {
        // PowerShell prints one path; whitespace and a trailing newline are
        // its own, and a path with spaces must survive intact. Nothing here
        // rebuilds a path: guessing %USERPROFILE%\\Documents was the bug that
        // put the alias in a file PowerShell never loads (OneDrive moves
        // Documents).
        let answered = "C:\\Users\\me\\OneDrive\\Dokumente\\WindowsPowerShell\\profile.ps1\r\n";
        assert_eq!(
            profile_path_from_output(answered),
            Some(std::path::PathBuf::from(
                "C:\\Users\\me\\OneDrive\\Dokumente\\WindowsPowerShell\\profile.ps1"
            ))
        );
        let with_spaces = "\n  C:\\Users\\a b\\Documents\\PowerShell\\profile.ps1  \n";
        assert_eq!(
            profile_path_from_output(with_spaces),
            Some(std::path::PathBuf::from(
                "C:\\Users\\a b\\Documents\\PowerShell\\profile.ps1"
            ))
        );
        // No answer at all: no path, and every surface stays silent.
        assert_eq!(profile_path_from_output("   \n\n"), None);
        assert_eq!(profile_path_from_output(""), None);
    }

    #[test]
    fn alias_detection_is_loose_but_correct() {
        assert!(alias_present("Set-Alias joy joy.exe"));
        assert!(alias_present("  set-alias   joy   C:\\tools\\joy.exe"));
        assert!(alias_present("Set-Alias -Name joy joy.exe"));
        assert!(alias_present("# comment\nSet-Alias joy joy.exe\n"));
        // Custom target (absolute path): still counts, we never overwrite it.
        assert!(alias_present(
            "Set-Alias -Name joy \"C:\\Program Files\\joy\\joy.exe\""
        ));

        // Not a joy alias / not an alias at all.
        assert!(!alias_present(""));
        assert!(!alias_present("Set-Alias joyful something"));
        assert!(!alias_present("# Set-Alias joy joy.exe"));
        assert!(!alias_present("Set-Alias joy")); // no target
        assert!(!alias_present("Get-Alias joy"));
    }

    #[test]
    fn append_payload_keeps_one_blank_line() {
        assert_eq!(append_payload(""), "Set-Alias joy joy.exe\n");
        assert_eq!(append_payload("x\n\n"), "Set-Alias joy joy.exe\n");
        assert_eq!(append_payload("x\n"), "\nSet-Alias joy joy.exe\n");
        assert_eq!(append_payload("x"), "\n\nSet-Alias joy joy.exe\n");
    }
}
