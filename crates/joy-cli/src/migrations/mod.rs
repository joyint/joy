// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The command line's part of joy-core's migrations (ADR JOY-0244-9E):
//! only what a migration needs of a terminal. Which migration waits for
//! a person, and what it does, is joy-core's
//! (`joy_core::migrations::waiting`); here is the question at the bottom
//! of the output and the passphrase after a yes. Nothing here is dated:
//! when no migration waits, [`offer`] has nothing to ask.

use std::io::{IsTerminal, Write};
use std::path::Path;

/// What [`offer`] came to.
pub enum Outcome {
    /// The project's data is from this joy.
    NothingWaits,
    /// The person said yes and the project is updated.
    Updated,
    /// The person said no, or the update did not go through; said why.
    NotNow,
    /// Nobody here can be asked: no terminal, or no person with a key.
    NobodyToAsk,
}

/// Ask whether the project's data from an older joy is to be updated,
/// as the last line on stderr in light orange, and on a yes (an empty
/// line is one) update it with the key of the person here: their session,
/// or the passphrase they are asked for next. ONLY a command that needs
/// the update calls this, and `joy update` (JOY-02CC-C1): it changes what
/// an older joy can read, so a person says yes to it every time.
pub fn offer(root: &Path) -> Outcome {
    let Some(waiting) = joy_core::migrations::waiting(root) else {
        return Outcome::NothingWaits;
    };
    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        return Outcome::NobodyToAsk;
    }
    let Ok(project) = joy_core::store::load_project(root) else {
        return Outcome::NobodyToAsk;
    };
    let Ok(person) = joy_core::identity::acting_human_key(root) else {
        return Outcome::NobodyToAsk;
    };
    let has_a_key = project
        .member_by_key(&person)
        .is_some_and(|member| member.verify_key.is_some());
    if !has_a_key {
        return Outcome::NobodyToAsk;
    }
    eprint!(
        "{} ",
        crate::color::hint_on_stderr(&format!("{} [Y/n]", waiting.question()))
    );
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err()
        || !crate::commands::update::says_yes(&answer)
    {
        return Outcome::NotNow;
    }
    let unlocked = match crate::auth_gate::unlock(root, &project, &person) {
        Ok(unlocked) => unlocked,
        Err(e) => {
            eprintln!("Not updated: {e}");
            return Outcome::NotNow;
        }
    };
    match joy_core::migrations::update_with(root, &person, &unlocked.keypair) {
        Ok(true) => Outcome::Updated,
        Ok(false) => Outcome::NobodyToAsk,
        Err(e) => {
            eprintln!("Not updated: {e}");
            Outcome::NotNow
        }
    }
}

/// For a command that `what` needs the update for ("Your own model for
/// claude"): ask, and `Ok(true)` when the project was updated and is to
/// be read again. `Ok(false)`: nothing waited. Otherwise the command
/// stops with the sentence that says what needs the update.
pub fn needed_for(root: &Path, what: &str) -> anyhow::Result<bool> {
    let needs = joy_core::migrations::needs_an_update(what);
    match offer(root) {
        Outcome::NothingWaits => Ok(false),
        Outcome::Updated => Ok(true),
        Outcome::NotNow => anyhow::bail!("{needs}"),
        Outcome::NobodyToAsk => anyhow::bail!("{needs} Run joy update at a terminal."),
    }
}
