// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Writes the forge CLIs a test finds instead of the person's own
//! (JOY-02BB-C7): `gh`, `glab` and `tea`, each one a program that is
//! installed and knows nobody. They are written once, at build time,
//! into this crate's OUT_DIR, so a test run creates nothing and two test
//! processes never write the same file.
//!
//! Unix only. Windows starts a program by its `.exe`, and a script named
//! `gh` is not one; there the directory stays empty and a test still
//! finds the person's CLI, which the crate documentation says.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let dir =
        PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("forge-clis");
    std::fs::create_dir_all(&dir).expect("the directory of the forge CLI stand-ins");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for name in ["gh", "glab", "tea"] {
            let path = dir.join(name);
            std::fs::write(
                &path,
                "#!/bin/sh\n# A forge CLI that is installed and signed in nowhere (JOY-02BB-C7).\nexit 1\n",
            )
            .expect("a forge CLI stand-in");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("an executable forge CLI stand-in");
        }
    }
    println!("cargo:rustc-env=JOY_TEST_ENV_FORGE_CLIS={}", dir.display());
}
