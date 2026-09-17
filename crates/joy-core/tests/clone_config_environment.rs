// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! A clone decides its proxy only AFTER git's own system config rule is
//! applied (JOY-028D-46, D1.11).
//!
//! libgit2 always adds the system-wide gitconfig; git skips it when
//! `GIT_CONFIG_NOSYSTEM` is true, and the one place that takes the
//! System level away from libgit2 is `vcs::forge::git_config_environment`.
//! Every entry into libgit2 goes through it first, and the proxy
//! decision is an entry like any other: `options_for` opens the default
//! config to read `remote.<name>.proxy`, `http.<url>.proxy` and
//! `http.proxy`. A clone is the one verb that reaches libgit2 without
//! going through `open`, so it is the one that can get this wrong, and
//! the proxy of a whole contact would then be decided from a file git
//! itself reads nothing of.
//!
//! The proof is a system gitconfig that names a proxy joy REFUSES: a
//! decision that read it says so by name and dials nothing, so the two
//! orders are told apart by the sentence and not by a timeout.
//!
//! Its own test binary: it changes the process environment and
//! libgit2's search path, both process state, and it has to be the
//! first git action of its process.

use std::fs;

use joy_core::vcs::forge::{self, Auth};
use joy_core::vcs::HostKind;

#[test]
fn a_clone_reads_no_proxy_from_a_system_config_git_would_not_read() {
    let home = tempfile::tempdir().unwrap();
    let system = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    fs::write(
        system.path().join("gitconfig"),
        "[http]\n\tproxy = socks5://127.0.0.1:1080\n",
    )
    .unwrap();
    std::env::set_var("HOME", home.path());
    std::env::set_var("XDG_CONFIG_HOME", home.path().join(".config"));
    std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    for name in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
    ] {
        std::env::remove_var(name);
    }
    // The levels are set on libgit2 itself, not through the
    // environment: on Windows the global config is also looked up under
    // USERPROFILE, where a CI runner keeps its own identity.
    unsafe {
        for level in [
            git2::ConfigLevel::Global,
            git2::ConfigLevel::XDG,
            git2::ConfigLevel::ProgramData,
        ] {
            git2::opts::set_search_path(level, home.path().to_str().unwrap()).unwrap();
        }
        git2::opts::set_search_path(git2::ConfigLevel::System, system.path().to_str().unwrap())
            .unwrap();
    }

    // The address is an http one, because a local path takes no proxy
    // at all and would never reach the decision, and it is a closed
    // port on the loopback interface, because nothing here may leave
    // this machine: port 1 answers "connection refused" at once, with
    // no name to resolve and no network.
    joy_core::vcs::contact::set_gaps("127.0.0.1=0,default=0");
    let dest = work.path().join("checkout");
    let failure = forge::clone(
        "http://127.0.0.1:1/forge.git",
        &Auth::LocalAs(HostKind::Background),
        &dest,
    )
    .expect_err("nothing listens on port 1");
    let text = format!("{failure:#}");
    assert!(
        !text.contains("SOCKS"),
        "the clone decided its proxy from a system config git reads nothing of: {text}"
    );
    // and it really got as far as the socket, which is what says the
    // decision was made and was "no proxy"
    assert!(
        !text.contains("proxy"),
        "no proxy was configured for this contact at all: {text}"
    );
    joy_core::vcs::contact::set_gaps("");
}
