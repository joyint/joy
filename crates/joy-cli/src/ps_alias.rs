// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Windows PowerShell alias hint (JOY-01C3-90).
//!
//! On Windows, the bare command `joy` resolves to
//! `C:\Windows\System32\joy.cpl` (Game Controllers) before the installed
//! `joy.exe`. A `Set-Alias joy joy.exe` line in the user's PowerShell
//! `$PROFILE` fixes interactive PowerShell sessions. This module detects
//! whether that alias is present and drives the hint surfaces (help tip,
//! first-run prompt, `joy update` prompt).
//!
//! The pure logic (alias regex, profile-path building, append payload, edition
//! detection from a process name) is cross-platform and unit-tested on every
//! OS. Only the OS glue (parent-process inspection, `$env:USERPROFILE`, file
//! IO) is `#[cfg(windows)]`; the public surface is a no-op off Windows.

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

/// The `$PROFILE` path for an edition, given the user's profile directory. This
/// mirrors how PowerShell itself resolves `$PROFILE` from `$env:USERPROFILE`.
fn profile_path(user_profile: &std::path::Path, edition: PsEdition) -> std::path::PathBuf {
    let sub = match edition {
        PsEdition::Core => "PowerShell",
        PsEdition::WindowsPowerShell => "WindowsPowerShell",
    };
    user_profile
        .join("Documents")
        .join(sub)
        .join("Microsoft.PowerShell_profile.ps1")
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
    /// is not PowerShell (cmd, a CI runner, unknown) -- in which case we emit
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
    /// present)`. `None` when the calling shell is not PowerShell or the user
    /// profile directory is unknown.
    fn target_profile() -> Option<(PathBuf, bool)> {
        let edition = parent_ps_edition()?;
        let user_profile = std::env::var_os("USERPROFILE")?;
        let path = profile_path(Path::new(&user_profile), edition);
        let present = std::fs::read_to_string(&path)
            .map(|c| alias_present(&c))
            .unwrap_or(false);
        Some((path, present))
    }

    /// Append the alias line to the profile, creating the file and its parent
    /// directory if needed. A freshly created file gets a UTF-8 BOM (Windows
    /// PowerShell 5.1 still expects one for profile files; PowerShell 7
    /// tolerates it).
    fn write_alias(path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let creating = !path.exists();
        let existing = std::fs::read_to_string(path).unwrap_or_default();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        if creating {
            file.write_all(&[0xEF, 0xBB, 0xBF])?;
        }
        file.write_all(append_payload(&existing).as_bytes())
    }

    /// Passive help tip: printed at the end of `joy help` / `joy -h` when the
    /// alias is missing. No prompt.
    pub fn print_help_tip() {
        if let Some((_, present)) = target_profile() {
            if !present {
                println!();
                println!(
                    "Tip: 'joy' alone opens Windows Game Controllers. To make 'joy' launch this tool:"
                );
                println!("  Add-Content $PROFILE \"{ALIAS_LINE}\"");
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
        let Some((path, present)) = target_profile() else {
            return;
        };
        if present {
            return;
        }
        println!();
        println!("Tip: 'joy' alone opens Windows Game Controllers. Add `{ALIAS_LINE}`");
        println!("to your PowerShell $PROFILE so 'joy' launches this tool.");
        match crate::prompt::ask_yn("Do it now?", true) {
            Ok(true) => match write_alias(&path) {
                Ok(()) => println!("Done. Restart your shell."),
                Err(e) => eprintln!("Could not update {}: {e}", path.display()),
            },
            _ => {}
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
    use std::path::Path;

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
    fn profile_paths_per_edition() {
        // Build the expected suffix with `join` so the comparison is by path
        // component, not by separator (this test runs on every OS).
        let home = Path::new(r"C:\Users\me");
        let core_suffix = Path::new("Documents")
            .join("PowerShell")
            .join("Microsoft.PowerShell_profile.ps1");
        let win_suffix = Path::new("Documents")
            .join("WindowsPowerShell")
            .join("Microsoft.PowerShell_profile.ps1");
        assert!(profile_path(home, PsEdition::Core).ends_with(&core_suffix));
        assert!(profile_path(home, PsEdition::WindowsPowerShell).ends_with(&win_suffix));
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
