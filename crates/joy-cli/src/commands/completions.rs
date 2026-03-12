// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::{bail, Result};
use clap_complete::{generate, Shell};
use std::io;

#[derive(clap::Args)]
#[command(after_help = "\
Dynamic completions (recommended, includes item/milestone ID completion):

  Bash:  source <(COMPLETE=bash joy)     # add to ~/.bashrc
  Zsh:   source <(COMPLETE=zsh joy)      # add to ~/.zshrc
  Fish:  source (COMPLETE=fish joy | psub)  # add to config.fish

Static completions (subcommands and flags only, no ID completion):

  eval \"$(joy completions bash)\"
  eval \"$(joy completions zsh)\"
  joy completions fish | source")]
pub struct CompletionsArgs {
    /// Target shell (bash, zsh, fish, powershell, elvish)
    shell: String,
}

pub fn run(args: CompletionsArgs, cmd: &mut clap::Command) -> Result<()> {
    let shell = match args.shell.to_lowercase().as_str() {
        "bash" => Shell::Bash,
        "zsh" => Shell::Zsh,
        "fish" => Shell::Fish,
        "powershell" => Shell::PowerShell,
        "elvish" => Shell::Elvish,
        _ => bail!(
            "unsupported shell '{}'. Supported: bash, zsh, fish, powershell, elvish",
            args.shell
        ),
    };

    generate(shell, cmd, "joy", &mut io::stdout());
    Ok(())
}
