// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! A command that names an AI member by its legacy form (JI-019D-46).
//!
//! An AI member is known by its name. It used to be written with more
//! around that name ([`joy_model::migrations::ai_member_name`]), and a command typed that
//! way keeps working: before the arguments are parsed, an argument that
//! is exactly such an id is replaced by the name, and one line says so.
//! This is the one place that does it, so no command ever sees the legacy
//! form.
//!
//! Only a whole argument is replaced. A legacy form inside a longer text
//! (a comment, a description) stays as the person wrote it.

use joy_core::migrations::ai_member_name;

/// The arguments with every AI member in the legacy form replaced by its name.
/// Says what it renamed on stderr, once per member, unless the output is
/// for a program.
pub fn modernise(raw: Vec<String>) -> Vec<String> {
    if !raw.iter().skip(1).any(|arg| ai_member_name::is_typed(arg)) {
        return raw;
    }
    let quiet = raw.iter().any(|arg| arg == "--json");
    let mut said: Vec<String> = Vec::new();
    raw.into_iter()
        .enumerate()
        .map(|(position, arg)| {
            if position == 0 || !ai_member_name::is_typed(&arg) {
                return arg;
            }
            let name = ai_member_name::read(&arg).to_string();
            if !quiet && !said.contains(&arg) {
                eprintln!("note: {arg} is now called {name}");
                said.push(arg);
            }
            name
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_whole_argument_that_is_an_legacy_form_becomes_the_name() {
        let older = ai_member_name::legacy_form("claude");
        assert_eq!(
            modernise(args(&["joy", "assign", "JOY-1", &older, "--json"])),
            args(&["joy", "assign", "JOY-1", "claude", "--json"])
        );
    }

    #[test]
    fn an_legacy_form_inside_a_text_stays_as_the_person_wrote_it() {
        let text = format!("mention {} here", ai_member_name::legacy_form("claude"));
        let raw = args(&["joy", "add", "task", &text]);
        assert_eq!(modernise(raw.clone()), raw);
    }
}
