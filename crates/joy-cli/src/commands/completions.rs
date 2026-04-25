// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::{bail, Result};
use clap_complete::{generate, Shell};
use std::io;

#[derive(clap::Args)]
#[command(after_help = "\
For bash, the default output is a dynamic, colon-aware wrapper around
clap_complete that supports item/milestone IDs and member IDs (including
'ai:tool@joy'-style values that bash would otherwise split at the colon).

Recommended setup:

  Bash:  source <(joy completions bash)         # add to ~/.bashrc
  Zsh:   source <(COMPLETE=zsh joy)             # add to ~/.zshrc
  Fish:  source (COMPLETE=fish joy | psub)      # add to config.fish

For fully case-insensitive completion (so 'p<TAB>' finds Peter.*) put

  set completion-ignore-case on

into ~/.inputrc; this is a readline setting and applies to all commands.

Use --static for the legacy bash output without colon handling and
without item/member ID completion.")]
pub struct CompletionsArgs {
    /// Target shell (bash, zsh, fish, powershell, elvish)
    shell: String,

    /// Emit the legacy static script (subcommands and flags only).
    /// Bash only; other shells already produce static output.
    #[arg(long)]
    static_only: bool,
}

pub fn run(args: CompletionsArgs, cmd: &mut clap::Command) -> Result<()> {
    let shell_lc = args.shell.to_lowercase();

    if shell_lc == "bash" && !args.static_only {
        print!("{}", BASH_DYNAMIC_COLON_AWARE);
        return Ok(());
    }

    let shell = match shell_lc.as_str() {
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

/// Bash dynamic completion that:
/// 1. Pre-merges tokens that bash split on `:` (default COMP_WORDBREAKS)
///    so values like `ai:claude@joy` look like one word to clap_complete.
/// 2. Calls the clap_complete bash protocol against the joy binary.
/// 3. Strips the `prefix:` portion from each candidate so bash inserts
///    only the post-colon suffix (the standard COMP_WORDBREAKS workaround).
///
/// zsh and fish do not split on `:` by default so they use the default
/// dynamic script via `COMPLETE=zsh joy` / `COMPLETE=fish joy`.
const BASH_DYNAMIC_COLON_AWARE: &str = r#"_joy_complete() {
    local IFS=$'\013'

    # Walk COMP_WORDS and merge any group separated by ':' into one word,
    # tracking how the original COMP_CWORD maps onto the merged list.
    local _words=()
    local _cword=0
    local _orig=$COMP_CWORD
    local _i=0
    while ((_i < ${#COMP_WORDS[@]})); do
        local _cur="${COMP_WORDS[_i]}"
        local _slot=${#_words[@]}
        ((_i == _orig)) && _cword=$_slot
        while ((_i + 1 < ${#COMP_WORDS[@]})); do
            local _next="${COMP_WORDS[_i+1]}"
            if [[ "$_cur" == *: || "$_next" == :* || "$_next" == ":" ]]; then
                _cur="${_cur}${_next}"
                ((_i++))
                ((_i == _orig)) && _cword=$_slot
            else
                break
            fi
        done
        _words+=("$_cur")
        ((_i++))
    done

    local _cur_word="${_words[_cword]}"

    local _SPACE=true
    if compopt +o nospace 2> /dev/null; then
        _SPACE=false
    fi

    # IFS=$'\013' is set above so the unquoted command substitution
    # below word-splits the clap_complete output into one entry per
    # candidate (clap_complete uses U+000B / VT as its delimiter).
    local _candidates=( $(
        _CLAP_IFS="$IFS" \
        _CLAP_COMPLETE_INDEX="$_cword" \
        _CLAP_COMPLETE_COMP_TYPE="${COMP_TYPE:-9}" \
        _CLAP_COMPLETE_SPACE="$_SPACE" \
        COMPLETE="bash" \
        joy -- "${_words[@]}"
    ) )
    if [[ $? -ne 0 ]]; then
        unset COMPREPLY
        return
    fi

    # Strip the colon-prefix from candidates so bash's already-typed
    # 'ai:' prefix is not duplicated when inserting.
    COMPREPLY=()
    local _strip=""
    if [[ "$_cur_word" == *:* ]]; then
        _strip="${_cur_word%:*}:"
    fi
    local _c
    for _c in "${_candidates[@]}"; do
        [[ -z "$_c" ]] && continue
        if [[ -n "$_strip" ]]; then
            COMPREPLY+=("${_c#$_strip}")
        else
            COMPREPLY+=("$_c")
        fi
    done

    if [[ $_SPACE == false ]] && [[ "${COMPREPLY-}" =~ [=/:]$ ]]; then
        compopt -o nospace
    fi
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -o nospace -o bashdefault -o nosort -F _joy_complete joy
else
    complete -o nospace -o bashdefault -F _joy_complete joy
fi
"#;
