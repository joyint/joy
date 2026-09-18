// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! Reading one running process's argument list, the way a person with
//! `ps` would (JOY-02A8-F4).
//!
//! J2's acceptance is "`ps` during any call shows no token", and the
//! two cases that prove it used to read `/proc/<pid>/cmdline` under a
//! plain `cfg(unix)` or `cfg(target_os = "linux")`. macOS has no
//! `/proc` at all, so on the one platform whose keychain this design
//! leans on the acceptance either failed for the wrong reason or was
//! quietly skipped. Here it is one helper with two readers, and both
//! cases keep running everywhere a token can travel. Windows has no
//! `/proc` and no `ps` either; there the argument list is what the
//! process object itself says, read with PowerShell, so the acceptance
//! holds on the third platform as well instead of failing to compile
//! (the Windows job of CI on 1bb6134 did exactly that).

/// One running process's argument list, its arguments separated by
/// single spaces. `None` while the process has not exec'd yet, or once
/// it is gone.
pub fn argv_of(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        let text = String::from_utf8_lossy(&raw).replace('\0', " ");
        (!text.trim().is_empty()).then_some(text)
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        // `-o args=` prints the argument list and no header, `-p` names
        // the one process, and `/bin/ps` by absolute path because the
        // cases empty PATH for the whole test binary. A pid that is
        // already gone exits non-zero with nothing, which is `None`
        // here and one more turn of the caller's poll.
        let out = joy_process::command("/bin/ps")
            .args(["-o", "args=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!text.is_empty()).then_some(text)
    }
    #[cfg(windows)]
    {
        // The absolute path for the same reason `/bin/ps` is absolute
        // above: the cases empty PATH. The process object's own
        // command line is what Task Manager shows; a pid that is gone
        // yields an empty answer, which is `None` here.
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
        let shell = format!("{root}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe");
        let out = joy_process::command(&shell)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!("(Get-CimInstance Win32_Process -Filter 'ProcessId={pid}').CommandLine"),
            ])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!text.is_empty()).then_some(text)
    }
}
