// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The forge CLIs joy reads a fact from: where their configuration
//! lives, and how a token is obtained from them (D2.4, decision 19).
//!
//! Two rules from the design hold here:
//!
//! - **The configuration discovery is per operating system.** Until now
//!   joy looked in `~/.config/<cli>` and nowhere else, so a Windows
//!   install of gh, a glab with `GLAB_CONFIG_DIR` set and every macOS
//!   install that follows XDG were invisible.
//! - **A foreign credential is obtained by spawning the CLI**, never by
//!   reading its store. That is decision 19, and it is also the only
//!   way the CLI's own refresh runs. In this wave the one source is
//!   `gh auth token`; D2.4 names `glab auth credential-helper` and
//!   `tea login helper get` for the `token` verb J3 builds.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use crate::config::{dedup, env_dir};

/// The files that may hold gh's configuration, most specific first.
pub fn gh_config_files() -> Vec<PathBuf> {
    gh_config_dirs()
        .into_iter()
        .map(|dir| dir.join("hosts.yml"))
        .collect()
}

/// gh's configuration directories (D2.4): `GH_CONFIG_DIR`,
/// `XDG_CONFIG_HOME/gh`, `%AppData%\GitHub CLI`, `~/.config/gh`.
pub fn gh_config_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = env_dir("GH_CONFIG_DIR") {
        dirs.push(dir);
    }
    if let Some(dir) = env_dir("XDG_CONFIG_HOME") {
        dirs.push(dir.join("gh"));
    }
    if cfg!(windows) {
        if let Some(dir) = env_dir("APPDATA") {
            dirs.push(dir.join("GitHub CLI"));
        }
    }
    if let Some(home) = env_dir("HOME") {
        dirs.push(home.join(".config/gh"));
    }
    dedup(dirs)
}

/// The files that may hold glab's configuration, most specific first.
pub fn glab_config_files() -> Vec<PathBuf> {
    glab_config_dirs()
        .into_iter()
        .map(|dir| dir.join("config.yml"))
        .collect()
}

/// glab's configuration directories (D2.4): `GLAB_CONFIG_DIR`,
/// `~/.config/glab-cli`, then XDG per platform including
/// `%LOCALAPPDATA%\glab-cli`.
pub fn glab_config_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = env_dir("GLAB_CONFIG_DIR") {
        dirs.push(dir);
    }
    if let Some(home) = env_dir("HOME") {
        dirs.push(home.join(".config/glab-cli"));
    }
    if let Some(dir) = env_dir("XDG_CONFIG_HOME") {
        dirs.push(dir.join("glab-cli"));
    }
    if cfg!(windows) {
        if let Some(dir) = env_dir("LOCALAPPDATA") {
            dirs.push(dir.join("glab-cli"));
        }
        if let Some(dir) = env_dir("APPDATA") {
            dirs.push(dir.join("glab-cli"));
        }
    }
    dedup(dirs)
}

/// The files that may hold tea's configuration, most specific first
/// (D2.4): XDG per platform, then the legacy `~/.tea/tea.yml`.
pub fn tea_config_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Some(dir) = env_dir("TEA_CONFIG_DIR") {
        files.push(dir.join("config.yml"));
    }
    if let Some(dir) = env_dir("XDG_CONFIG_HOME") {
        files.push(dir.join("tea/config.yml"));
    }
    if cfg!(windows) {
        if let Some(dir) = env_dir("APPDATA") {
            files.push(dir.join("tea/config.yml"));
        }
        if let Some(dir) = env_dir("LOCALAPPDATA") {
            files.push(dir.join("tea/config.yml"));
        }
    }
    if let Some(home) = env_dir("HOME") {
        files.push(home.join(".config/tea/config.yml"));
        if cfg!(target_os = "macos") {
            files.push(home.join("Library/Application Support/tea/config.yml"));
        }
        files.push(home.join(".tea/tea.yml"));
    }
    dedup(files)
}

/// The first of these files that exists and reads.
pub fn first_readable(files: &[PathBuf]) -> Option<(PathBuf, String)> {
    files.iter().find_map(|path| {
        std::fs::read_to_string(path)
            .ok()
            .map(|text| (path.clone(), text))
    })
}

/// How long a foreign CLI has to answer. D2.3 gives `token` 30 s, "gh
/// alone allows 60 s per keyring read", and this call sits inside a
/// verb whose own deadline is already running.
const CLI_TIMEOUT: Duration = Duration::from_secs(20);

/// The token gh holds for a host, by spawning gh (decision 19).
///
/// `gh auth token` prints the token on stdout and nothing else. It is
/// never an argument to anything, so no process list can carry it.
pub fn gh_token(host: &str, login: Option<&str>) -> Option<String> {
    let mut command = joy_process::command("gh");
    command.args(["auth", "token", "--hostname", host]);
    if let Some(login) = login {
        command.args(["--user", login]);
    }
    run_for_stdout(command)
}

/// Run a CLI and take its stdout, bounded. Stdin is closed so a CLI
/// that would ask something fails instead of waiting forever.
fn run_for_stdout(mut command: std::process::Command) -> Option<String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + CLI_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                break;
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    }
    let output = child.wait_with_output().ok()?;
    let token = String::from_utf8(output.stdout).ok()?;
    let token = token.trim().to_string();
    (!token.is_empty()).then_some(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The discovery order is the one D2.4 names, and the explicit
    /// variable always wins so a test and a workstation image can point
    /// at their own file.
    #[test]
    fn the_config_order_per_cli_is_the_one_the_design_names() {
        let gh = gh_config_dirs();
        let glab = glab_config_dirs();
        let tea = tea_config_files();
        // nothing is empty on a machine with a HOME, and every entry is
        // distinct
        assert!(!gh.is_empty() && !glab.is_empty() && !tea.is_empty());
        for list in [&gh, &glab] {
            let mut sorted = list.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), list.len());
        }
        // the legacy tea file is last, after the XDG ones
        if let Some(home) = env_dir("HOME") {
            let legacy = home.join(".tea/tea.yml");
            if let Some(index) = tea.iter().position(|p| *p == legacy) {
                assert_eq!(index, tea.len() - 1);
            }
        }
    }
}
