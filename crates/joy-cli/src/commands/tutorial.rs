// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use std::io::Write;
use std::process::{Command, Stdio};

use termimad::MadSkin;

const TUTORIAL: &str = include_str!("../../../../docs/user/Tutorial.md");

pub fn run() -> Result<()> {
    let width = crate::color::terminal_width();
    let skin = MadSkin::default_dark();
    let formatted = skin.area_text(TUTORIAL, &termimad::Area::new(0, 0, width as u16, u16::MAX));

    // Try pager in order: $PAGER, less, more
    let pager = std::env::var("PAGER").ok().unwrap_or_default();

    let pagers = if pager.is_empty() {
        vec!["less -R", "more"]
    } else {
        vec![pager.as_str(), "less -R", "more"]
    };

    let output = formatted.to_string();

    for p in &pagers {
        let parts: Vec<&str> = p.split_whitespace().collect();
        let (cmd, args) = match parts.split_first() {
            Some((c, a)) => (*c, a),
            None => continue,
        };

        let mut child = match Command::new(cmd).args(args).stdin(Stdio::piped()).spawn() {
            Ok(c) => c,
            Err(_) => continue,
        };

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(output.as_bytes());
        }

        let _ = child.wait();
        return Ok(());
    }

    // Fallback: print directly
    print!("{output}");
    Ok(())
}
