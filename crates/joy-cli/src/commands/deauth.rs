// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use joy_core::auth::session;
use joy_core::store;
use joy_core::vcs::Vcs;

/// `joy deauth` — end the current session.
pub fn run() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let project_id = session::project_id(&root)?;
    let member = joy_core::vcs::default_vcs().user_email()?;
    session::remove_session(&project_id, &member)?;

    println!("Session ended.");

    Ok(())
}
