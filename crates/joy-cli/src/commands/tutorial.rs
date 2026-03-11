// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use std::io::Write;
use std::process::{Command, Stdio};

const TUTORIAL: &str = include_str!("../../../../docs/user/Tutorial.md");

pub fn run() -> Result<()> {
    // Try pager in order: $PAGER, less, more
    let pager = std::env::var("PAGER").ok().unwrap_or_default();

    let pagers = if pager.is_empty() {
        vec!["less", "more"]
    } else {
        vec![pager.as_str(), "less", "more"]
    };

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
            let _ = stdin.write_all(TUTORIAL.as_bytes());
        }

        let _ = child.wait();
        return Ok(());
    }

    // Fallback: print directly
    print!("{TUTORIAL}");
    Ok(())
}
