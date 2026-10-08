// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! `ai:claude@joy` is now called `claude` (JI-019D-46).
//!
//! An AI member used to be written `ai:<name>@joy`. With member files it
//! is known by its name alone. A command typed the old way keeps working:
//! before the arguments are parsed, an argument that is exactly an old
//! id is replaced by the name, and one line says so. This is the one
//! place that does it, so no command has to know the old spelling.
//!
//! Only a whole argument is replaced. An old id inside a longer text (a
//! comment, a description) stays as the person wrote it. A project from
//! before the member files still uses the old ids, and there nothing is
//! touched.

use std::path::PathBuf;

use joy_core::model::project::{ai_member_name, MemberLayout};
use joy_core::store;

fn is_old_id(arg: &str) -> bool {
    arg.len() > "ai:@joy".len()
        && arg.starts_with("ai:")
        && arg.ends_with("@joy")
        && !arg.contains(char::is_whitespace)
}

/// Where the command will run: `-w`/`--working-dir`, the environment,
/// else here.
fn working_dir(raw: &[String]) -> Option<PathBuf> {
    let mut args = raw.iter().skip(1);
    while let Some(arg) = args.next() {
        if arg == "-w" || arg == "--working-dir" {
            return args.next().map(PathBuf::from);
        }
        if let Some(path) = arg.strip_prefix("--working-dir=") {
            return Some(PathBuf::from(path));
        }
    }
    std::env::var_os("JOY_WORKING_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
}

/// The arguments with every old AI member id replaced by its name, when
/// the project here names its members the new way. Says what it renamed
/// on stderr, once per member, unless the output is for a program.
pub fn modernise(raw: Vec<String>) -> Vec<String> {
    if !raw.iter().skip(1).any(|arg| is_old_id(arg)) {
        return raw;
    }
    let names_members_by_name = working_dir(&raw)
        .and_then(|dir| store::find_project_root(&dir))
        .and_then(|root| store::load_project(&root).ok())
        .is_some_and(|project| project.member_layout() == MemberLayout::Files);
    if !names_members_by_name {
        return raw;
    }
    let quiet = raw.iter().any(|arg| arg == "--json");
    let mut said: Vec<String> = Vec::new();
    raw.into_iter()
        .enumerate()
        .map(|(position, arg)| {
            if position == 0 || !is_old_id(&arg) {
                return arg;
            }
            let name = ai_member_name(&arg).to_string();
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

    #[test]
    fn only_a_whole_argument_that_is_an_old_id_counts() {
        assert!(is_old_id("ai:claude@joy"));
        assert!(is_old_id("ai:copilot-chat@joy"));
        assert!(!is_old_id("claude"));
        assert!(!is_old_id("ai:@joy"));
        assert!(!is_old_id("ask ai:claude@joy to look"));
        assert!(!is_old_id("horst@joydev.com"));
    }

    #[test]
    fn without_an_old_id_the_arguments_come_back_untouched_and_no_project_is_read() {
        let raw: Vec<String> = ["joy", "add", "task", "mention ai:claude@joy here"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(modernise(raw.clone()), raw);
    }
}
