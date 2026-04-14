// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT
//
// Minimal interactive prompt helpers for terminal-only flows like the
// `joy` welcome wizard. Not a general-purpose TUI - just enough to ask
// for a yes/no answer or a line of text with a default.

use std::io::{self, BufRead, IsTerminal, Write};

/// True if both stdin and stdout are connected to a terminal.
pub fn is_interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// Ask a yes/no question. Returns the default on empty input.
pub fn ask_yn(question: &str, default: bool) -> io::Result<bool> {
    let hint = if default { "Y/n" } else { "y/N" };
    loop {
        print!("{question} ({hint}) ");
        io::stdout().flush()?;
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line)? == 0 {
            return Ok(default);
        }
        match line.trim().to_ascii_lowercase().as_str() {
            "" => return Ok(default),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("Please answer y or n."),
        }
    }
}

/// Ask for a line of text. Empty input returns the default, if any.
pub fn ask_text(question: &str, default: Option<&str>) -> io::Result<String> {
    loop {
        match default {
            Some(d) => print!("{question} ({d}) "),
            None => print!("{question} "),
        }
        io::stdout().flush()?;
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line)? == 0 {
            return Ok(default.unwrap_or("").to_string());
        }
        let answer = line.trim();
        if answer.is_empty() {
            if let Some(d) = default {
                return Ok(d.to_string());
            }
            println!("A value is required.");
            continue;
        }
        return Ok(answer.to_string());
    }
}
