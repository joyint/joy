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

/// The yes/no loop, over any reader and any writer.
///
/// It exists in this shape for the questions that interrupt a command
/// whose stdout is an answer: the host key question of D1.4a asks in
/// the middle of a fetch, so it asks on STDERR, and the fetch keeps
/// stdout. And it exists in this shape so that a question a person
/// answers can be proved without a terminal: the wording, the yes, the
/// no, the empty line and the closed stdin are all cases.
///
/// Returns the default on an empty line and on a closed stdin.
pub fn yes_or_no(
    question: &str,
    default: bool,
    input: &mut impl io::BufRead,
    out: &mut impl io::Write,
) -> io::Result<bool> {
    let hint = if default { "Y/n" } else { "y/N" };
    loop {
        write!(out, "{question} ({hint}) ")?;
        out.flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Ok(default);
        }
        match line.trim().to_ascii_lowercase().as_str() {
            "" => return Ok(default),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => writeln!(out, "Please answer y or n.")?,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn answered(typed: &str, default: bool) -> (bool, String) {
        let mut input = typed.as_bytes();
        let mut out: Vec<u8> = Vec::new();
        let answer = yes_or_no("Trust this host key?", default, &mut input, &mut out).unwrap();
        (answer, String::from_utf8(out).unwrap())
    }

    /// Every answer a person can give, including the two that are not
    /// answers: an empty line and a closed stdin both take the default,
    /// which for the host key question of D1.4a is NO.
    #[test]
    fn a_yes_no_question_takes_every_answer_a_person_gives() {
        assert!(answered("y\n", false).0);
        assert!(answered("YES\n", false).0);
        assert!(!answered("n\n", true).0);
        assert!(!answered("no\n", true).0);
        assert!(answered("\n", true).0, "an empty line takes the default");
        assert!(!answered("\n", false).0);
        assert!(!answered("", false).0, "a closed stdin takes the default");
        assert!(answered("", true).0);
    }

    /// Anything else is asked again, and the hint says which way the
    /// default falls.
    #[test]
    fn an_answer_that_is_neither_is_asked_again() {
        let (answer, seen) = answered("maybe\ny\n", false);
        assert!(answer);
        assert!(seen.contains("Please answer y or n."), "{seen}");
        assert!(seen.contains("Trust this host key? (y/N)"), "{seen}");
        let (_, seen) = answered("\n", true);
        assert!(seen.contains("(Y/n)"), "{seen}");
    }
}
