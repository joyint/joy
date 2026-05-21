// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use clap::Args;

// The canonical Tutorial lives at docs/user/Tutorial.md at the
// repo root. We ship an in-crate copy at crates/joy-cli/docs/
// Tutorial.md because `cargo package` builds the crate in isolation
// and cannot reach files outside the crate root. The two files must
// stay byte-identical; `just sync-tutorial` refreshes the copy and a
// unit test below catches drift. See JOY-017F-FD.
const TUTORIAL: &str = include_str!("../../docs/user/Tutorial.md");

#[derive(Args)]
pub struct TutorialArgs {
    /// Browse the tutorial via a chapter / subchapter menu (TTY only).
    #[arg(short = 'i', long)]
    interactive: bool,
}

pub fn run(args: TutorialArgs) -> Result<()> {
    // Rendering and interactive browsing live in joy_core::tutorial so
    // every Joyint CLI shares one implementation. See JOY-019F-50.
    joy_core::tutorial::run_markdown(TUTORIAL, args.interactive, true)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    /// The Tutorial lives in two places at once: docs/user/Tutorial.md
    /// is the canonical doc, crates/joy-cli/docs/user/Tutorial.md is
    /// shipped inside the crate so cargo package can find it. They
    /// must stay byte-identical. If this test fails, run
    /// `just sync-tutorial` from the repo root.
    #[test]
    fn in_crate_tutorial_matches_canonical() {
        let canonical = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/user/Tutorial.md"
        ));
        let shipped = super::TUTORIAL;
        assert_eq!(
            canonical, shipped,
            "crates/joy-cli/docs/user/Tutorial.md is out of sync with \
             docs/user/Tutorial.md. Run `just sync-tutorial`."
        );
    }
}
