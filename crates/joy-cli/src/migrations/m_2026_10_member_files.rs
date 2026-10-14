// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The command line's half of bringing a project from before the member
//! files over (JI-019D-46, JOY-02D5-55): the passphrase `joy update` asks
//! for, the sentence that says why, and the row it reports. The work
//! itself is joy-core's `migrations::m_2026_10_member_files`.
//!
//! A migration under ADR JOY-0244-9E: delete this module, joy-core's
//! module of the same name and the two calls in `commands/update.rs` once
//! no project from before is left.

use std::path::Path;

use joy_core::auth::IdentityKeypair;

/// The section and the words of the row `joy update` reports.
pub const SECTION: &str = "Members";
pub const MOVED: &str = "moved to their own files";
pub const WAITS: &str = "still in project.yaml: run joy update at a terminal, or with --passphrase";

/// What [`bring_over`] did.
pub enum Outcome {
    /// The project keeps its members in files already, or has none.
    NothingToDo,
    /// Done now: one file per member, signed by the person here.
    Moved,
    /// Still to do: nobody is signed in here and nobody could be asked
    /// (no terminal, no passphrase given), or the passphrase was wrong.
    WaitsForSignIn,
}

/// The one sentence `joy update` says before it asks for the passphrase.
const WHY_THE_PASSPHRASE: &str = "Members move to their own files; your passphrase signs that.";

/// Bring a project from before the member files over (JI-019D-46). ONLY
/// `joy update` calls this (JOY-02CC-C1): it changes what an older joy
/// can read, so it happens when a person asks for an update, never as a
/// side effect of a sign-in or of any other command. The key that signs
/// what the AI members may do is the one in the session of the person
/// signed in at this terminal. Where nobody is signed in, `joy update`
/// asks for the passphrase itself, with one sentence that says why, and
/// signs in with it the way `joy auth` does (operator, 2026-10-14: an
/// update, then `joy auth`, then the update again was nobody's idea of
/// a way). It asks only where somebody can answer: at a terminal, or
/// with a passphrase given. Otherwise the project stays as it is and
/// keeps working as it is.
pub fn bring_over(root: &Path, at_a_terminal: bool) -> Outcome {
    use std::io::IsTerminal;
    if !joy_core::migrations::m_2026_10_member_files::pending(root) {
        return Outcome::NothingToDo;
    }
    let Ok(project) = joy_core::store::load_project(root) else {
        return Outcome::WaitsForSignIn;
    };
    let Ok(member_key) = joy_core::identity::acting_human_key(root) else {
        return Outcome::WaitsForSignIn;
    };
    let keypair = match crate::auth_gate::session_seed(root, &project, &member_key) {
        Some(seed) => IdentityKeypair::from_seed(&seed),
        None => {
            let can_answer = at_a_terminal && std::io::stdin().is_terminal();
            if !crate::auth_gate::passphrase_given() && !can_answer {
                return Outcome::WaitsForSignIn;
            }
            if !crate::auth_gate::passphrase_given() {
                eprintln!("{WHY_THE_PASSPHRASE}");
            }
            match crate::auth_gate::unlock(root, &project, &member_key) {
                Ok(unlocked) => unlocked.keypair,
                Err(e) => {
                    eprintln!("Members not moved: {e}");
                    return Outcome::WaitsForSignIn;
                }
            }
        }
    };
    match joy_core::migrations::m_2026_10_member_files::migrate(root, &member_key, &keypair) {
        Ok(true) => Outcome::Moved,
        Ok(false) => Outcome::WaitsForSignIn,
        Err(e) => {
            eprintln!("Warning: this project's members could not be moved to their own files: {e}");
            Outcome::WaitsForSignIn
        }
    }
}
