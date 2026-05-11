// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

mod color;
mod commands;
mod complete;
mod crypt_session;
mod editor;
mod effort;
mod forge;
mod output;
mod prompt;
mod update_registry;
mod version_bump;

use std::io::IsTerminal;

use clap::{CommandFactory, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "joy",
    version,
    infer_subcommands = true,
    about = "Terminal-native product management",
    // clap 4.5 has no native subcommand grouping. The custom template
    // below omits {subcommands} so we can render a grouped list by
    // hand in `after_help`. Discoverability is preserved: every
    // subcommand still parses, completes, and replies to
    // `joy <cmd> --help`.
    help_template = "\
{about-with-newline}
Usage: {usage}

Options:
{options}

{after-help}",
    after_help = "\
Core Commands:
  init       Initialize a new Joy project
  add        Create a new item
  ls         List items
  show       Show item details
  edit       Modify an existing item
  status     Change item status
  comment    Add a comment to an item
  deps       Manage dependencies
  milestone  Manage milestones
  log        Show change history for items

Shortcuts:
  start   Set item status to in-progress
  submit  Set item status to review
  close   Set item status to closed
  reopen  Reopen a closed or deferred item

Discovery & Reporting:
  find     Search items by text
  roadmap  Show milestone roadmap (alias for ls --tree --group milestone)
  assign   Assign or unassign items
  rm       Delete an item
  release  Show release notes for a version

Confidentiality:
  auth    Authenticate (enter passphrase to start a session)
  deauth  End the current session
  crypt   Manage Crypt zones, items, paths, and grants

Project & Members:
  project  View or edit project metadata
  config   Show or modify configuration
  ai       AI tool integration

Maintenance:
  update       Update the joy binary and sync this repo's joy-managed state
  completions  Generate shell completions
  tutorial     Read the Joy tutorial

Quick start:
  joy init                              Set up a new project
  joy add task \"Fix login bug\"          Create an item
  joy ls                                List all items
  joy start IT-0001                     Start working on it
  joy                                   Show the board

Run 'joy tutorial' for the full guide.\n"
)]
pub(crate) struct Cli {
    /// Run as if joy was started in <PATH>
    #[arg(
        short = 'w',
        long = "working-dir",
        global = true,
        value_name = "PATH",
        value_hint = clap::ValueHint::DirPath
    )]
    working_dir: Option<std::path::PathBuf>,

    /// Show all items on the board (no limit per column)
    #[arg(short, long)]
    all: bool,

    /// Reverse sort order
    #[arg(short, long)]
    reverse: bool,

    /// Machine-readable JSON output.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new Joy project
    Init(commands::init::InitArgs),
    /// Create a new item
    Add(commands::add::AddArgs),
    /// List items
    Ls(commands::ls::LsArgs),
    /// Show item details
    Show(commands::show::ShowArgs),
    /// Modify an existing item
    Edit(commands::edit::EditArgs),
    /// Change item status
    Status(commands::status::StatusArgs),
    /// Delete an item
    Rm(commands::rm::RmArgs),
    /// Add a comment to an item
    Comment(commands::comment::CommentArgs),
    /// Manage dependencies
    Deps(commands::deps::DepsArgs),
    /// Manage milestones
    Milestone(commands::milestone::MilestoneArgs),
    /// View or edit project metadata
    Project(commands::project::ProjectArgs),
    /// Assign or unassign items
    Assign(commands::assign::AssignArgs),
    /// Show change history for items
    Log(commands::log::LogArgs),
    /// Generate shell completions
    Completions(commands::completions::CompletionsArgs),
    /// Read the Joy tutorial
    Tutorial,
    /// Show milestone roadmap (alias for ls --tree --group milestone)
    Roadmap(RoadmapArgs),
    /// Shortcut: set item status to in-progress
    Start(ShortcutArgs),
    /// Shortcut: set item status to review
    Submit(ShortcutArgs),
    /// Shortcut: set item status to closed
    Close(ShortcutArgs),
    /// Shortcut: set item status back to open
    Reopen(ShortcutArgs),
    /// Search items by text
    Find(commands::find::FindArgs),
    /// Show release notes for a version
    Release(commands::release::ReleaseArgs),
    /// Show the board (default when no command given)
    #[command(hide = true)]
    Board(BoardArgs),
    /// Show or modify configuration
    Config(commands::config::ConfigArgs),
    /// AI tool integration
    Ai(commands::ai::AiArgs),
    /// Authenticate (enter passphrase to start a session)
    Auth(commands::auth::AuthArgs),
    /// End the current session
    Deauth(commands::deauth::DeauthArgs),
    /// Manage Crypt zones, items, paths, and grants
    Crypt(commands::crypt::CryptArgs),
    /// Internal: Git merge driver helpers (invoked via .gitattributes).
    Merge(commands::merge::MergeArgs),
    /// Update the joy binary and sync this repo's joy-managed state.
    Update(commands::update::UpdateArgs),
}

#[derive(clap::Args)]
pub(crate) struct BoardArgs {
    #[command(flatten)]
    pub filter: commands::filter_args::FilterArgs,

    /// Compact output: emoji-only or abbreviations
    #[arg(short = 'S', long)]
    pub short: bool,

    /// Show all items (no limit per status group)
    #[arg(short, long)]
    pub all: bool,

    /// Reverse sort order (oldest first instead of newest first)
    #[arg(short, long)]
    pub reverse: bool,
}

#[derive(clap::Args)]
struct RoadmapArgs {
    /// Show all items (including closed and deferred)
    #[arg(short, long)]
    all: bool,

    /// Compact output: emoji-only or abbreviations
    #[arg(short = 'S', long)]
    short: bool,
}

/// Rewrite `joy <cmd...> help` to `joy <cmd...> --help` so users coming
/// from AWS/gcloud-style CLIs (where `help` is a subcommand at every
/// level) get the expected behaviour. The rewrite is conservative: it
/// only fires when the trailing `help` follows a chain of valid clap
/// subcommands. This way positional arguments that happen to be the
/// literal string `help` (e.g. `joy add task help`) are not stolen.
fn rewrite_trailing_help(mut args: Vec<String>, root: &clap::Command) -> Vec<String> {
    if args.last().map(|s| s.as_str()) != Some("help") {
        return args;
    }
    let mut current = root;
    let mut idx = 1;
    let last = args.len() - 1;
    while idx < last {
        match current.find_subcommand(&args[idx]) {
            Some(sub) => {
                current = sub;
                idx += 1;
            }
            None => return args,
        }
    }
    if idx == last {
        args[last] = "--help".to_string();
    }
    args
}

#[derive(clap::Args)]
struct ShortcutArgs {
    /// Item ID (e.g. IT-0001)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(complete::complete_item_id))]
    id: String,
}

/// Detect a binary-version mismatch against the per-clone marker and,
/// when auto-sync is enabled, run the repo sync. Best-effort: any error
/// here is swallowed; never blocks a user-facing command.
///
/// Skipped entirely for `joy update` itself, which runs the sync
/// explicitly with full output, and for `joy completions` which can
/// be invoked outside a project root.
fn auto_sync_repo() {
    let cwd = match std::env::current_dir() {
        Ok(c) => c,
        Err(_) => return,
    };
    let root = match joy_core::store::find_project_root(&cwd) {
        Some(r) => r,
        None => return,
    };

    let current = commands::update::CURRENT_VERSION;

    // Downgrade guard: if the repo was last synced by a newer joy
    // binary, refuse to touch anything and warn once. Running an old
    // binary against newer joy-managed files risks dropping schema
    // fields and rolling templates back. See JOY-016B-A1.
    if let Some(marker) = update_registry::marker_ahead_of(&root, current) {
        eprintln!(
            "warning: this repo was last synced with joy {marker}; you are running joy {current}.\n\
             Update joy before continuing to avoid downgrading repo state."
        );
        return;
    }

    // Always reassert lazy activation (cheap and idempotent).
    let _ = joy_core::init::ensure_lazy_activation(&root);

    // Honour the auto-sync toggle from .joy/config.yaml.
    let config = joy_core::store::load_config();
    if !config.auto_sync {
        return;
    }

    match joy_core::init::last_sync_version(&root) {
        Some(v) if v == current => {} // already in sync
        recorded => {
            // Run the full sync (lazy-activation + auth update + ai
            // update + stamp marker). Each routine prints its own
            // summary; the trailing one-liner ties them together so the
            // user / AI can spot the version transition.
            let _ = commands::update::run_full_sync(&root);
            let prev = recorded.unwrap_or_else(|| "(never)".to_string());
            eprintln!("joy {current}: synced this repo (previous marker: {prev}).");
        }
    }
}

fn main() -> anyhow::Result<()> {
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();

    let raw: Vec<String> = std::env::args().collect();
    let cli = Cli::parse_from(rewrite_trailing_help(raw, &Cli::command()));

    // Honour -w / --working-dir BEFORE anything that depends on cwd
    // (auto-sync, load_config, find_project_root). The target must be
    // an existing Joy project; otherwise we bail loudly so the user
    // does not run a subcommand against the wrong tree.
    if let Some(ref path) = cli.working_dir {
        let canon = std::fs::canonicalize(path)
            .map_err(|e| anyhow::anyhow!("--working-dir {}: {e}", path.display()))?;
        if joy_core::store::find_project_root(&canon).is_none() {
            anyhow::bail!("--working-dir {}: not a Joy project", canon.display());
        }
        std::env::set_current_dir(&canon)
            .map_err(|e| anyhow::anyhow!("--working-dir {}: {e}", canon.display()))?;
    }

    // Install --json mode before any subcommand runs, so config (which
    // returns early below) and others all see the same flag.
    output::set_mode(if cli.json {
        output::OutputMode::Json
    } else {
        output::OutputMode::Display
    });

    // Config subcommand handles its own validation, run it before load_config
    // to avoid duplicate warnings for invalid config state.
    if let Some(Commands::Config(args)) = cli.command {
        return commands::config::run(args);
    }

    // JOY-0162 / JOY-0164-B5: keep the merge-driver registration and the
    // per-clone version marker in sync with the current binary.
    // Best-effort, must never fail a user-facing command.
    //
    // Skip for `joy update`: that subcommand runs the sync itself
    // (with full output), and `joy update --check` must not write
    // anything.
    if !matches!(cli.command, Some(Commands::Update(_))) {
        auto_sync_repo();
    }

    let mut config = joy_core::store::load_config();

    // Extract --short from subcommands that support it
    let short_override = match &cli.command {
        None => false, // default board uses cli-level args handled below
        Some(Commands::Board(a)) => a.short,
        Some(Commands::Ls(a)) => a.short,
        Some(Commands::Show(a)) => a.short,
        Some(Commands::Roadmap(a)) => a.short,
        _ => false,
    };
    if short_override {
        config.output.short = true;
    }
    color::init(&config.output);

    let show_fortune = matches!(
        &cli.command,
        None | Some(Commands::Ls(_)) | Some(Commands::Roadmap(_)) | Some(Commands::Show(_))
    );

    let result = match cli.command {
        Some(Commands::Init(args)) => commands::init::run(args),
        Some(Commands::Add(args)) => commands::add::run(args),
        Some(Commands::Ls(args)) => commands::ls::run(args),
        Some(Commands::Show(args)) => commands::show::run(args),
        Some(Commands::Edit(args)) => commands::edit::run(args),
        Some(Commands::Status(args)) => commands::status::run(args),
        Some(Commands::Rm(args)) => commands::rm::run(args),
        Some(Commands::Comment(args)) => commands::comment::run(args),
        Some(Commands::Deps(args)) => commands::deps::run(args),
        Some(Commands::Milestone(args)) => commands::milestone::run(args),
        Some(Commands::Project(args)) => commands::project::run(args),
        Some(Commands::Assign(args)) => commands::assign::run(args),
        Some(Commands::Log(args)) => commands::log::run(args),
        Some(Commands::Completions(args)) => commands::completions::run(args, &mut Cli::command()),
        Some(Commands::Tutorial) => commands::tutorial::run(),
        Some(Commands::Roadmap(args)) => commands::ls::run(commands::ls::LsArgs::roadmap(args.all)),
        Some(Commands::Start(args)) => commands::status::run(commands::status::StatusArgs::new(
            args.id,
            "in-progress".to_string(),
        )),
        Some(Commands::Submit(args)) => commands::status::run(commands::status::StatusArgs::new(
            args.id,
            "review".to_string(),
        )),
        Some(Commands::Close(args)) => commands::status::run(commands::status::StatusArgs::new(
            args.id,
            "closed".to_string(),
        )),
        Some(Commands::Reopen(args)) => commands::status::run(commands::status::StatusArgs::new(
            args.id,
            "open".to_string(),
        )),
        Some(Commands::Find(args)) => commands::find::run(args),
        Some(Commands::Release(args)) => commands::release::run(args),
        Some(Commands::Board(args)) => commands::board::run(args),
        Some(Commands::Config(_)) => unreachable!("handled above"),
        Some(Commands::Ai(args)) => commands::ai::run(args),
        Some(Commands::Auth(args)) => commands::auth::run(args),
        Some(Commands::Deauth(args)) => commands::deauth::run(args),
        Some(Commands::Crypt(args)) => commands::crypt::run(args),
        Some(Commands::Merge(args)) => commands::merge::run(args),
        Some(Commands::Update(args)) => commands::update::run(args),
        None => commands::board::run(BoardArgs {
            filter: commands::filter_args::FilterArgs::default(),
            short: false,
            all: cli.all,
            reverse: cli.reverse,
        }),
    };

    if show_fortune
        && result.is_ok()
        && config.output.fortune
        && !output::is_json()
        && std::io::stdout().is_terminal()
    {
        if let Some(text) = joy_core::fortune::fortune(config.output.fortune_category.as_ref(), 0.2)
        {
            eprintln!("\n\x1b[2m{text}\x1b[0m");
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewrite(args: &[&str]) -> Vec<String> {
        let owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        rewrite_trailing_help(owned, &Cli::command())
    }

    #[test]
    fn help_after_leaf_command_becomes_double_dash_help() {
        assert_eq!(rewrite(&["joy", "ls", "help"]), &["joy", "ls", "--help"]);
        assert_eq!(
            rewrite(&["joy", "show", "help"]),
            &["joy", "show", "--help"]
        );
        assert_eq!(
            rewrite(&["joy", "board", "help"]),
            &["joy", "board", "--help"]
        );
    }

    #[test]
    fn help_after_nested_subcommand_becomes_double_dash_help() {
        assert_eq!(
            rewrite(&["joy", "project", "member", "help"]),
            &["joy", "project", "member", "--help"]
        );
    }

    #[test]
    fn help_alone_is_left_for_clap_to_handle() {
        // 'joy help' has no preceding subcommand; clap routes this to its
        // built-in top-level help. Rewrite would change it to --help which
        // is equivalent, so allowing the rewrite is harmless.
        assert_eq!(rewrite(&["joy", "help"]), &["joy", "--help"]);
    }

    #[test]
    fn help_as_value_after_positional_is_not_rewritten() {
        // `joy add task help` means "create a task titled help" -- the
        // rewriter must not steal the title. After 'add' clap expects
        // positionals (type, title) not subcommands, so the walker
        // stops at 'task' and refuses to rewrite.
        assert_eq!(
            rewrite(&["joy", "add", "task", "help"]),
            &["joy", "add", "task", "help"]
        );
    }

    #[test]
    fn long_dashed_options_pass_through_untouched() {
        assert_eq!(rewrite(&["joy", "ls", "--mine"]), &["joy", "ls", "--mine"]);
        assert_eq!(
            rewrite(&["joy", "ls", "-T", "bug"]),
            &["joy", "ls", "-T", "bug"]
        );
    }

    /// Every non-hidden subcommand from the Commands enum must appear
    /// in the grouped command list rendered by `joy --help`. The
    /// help_template drops clap's native subcommand block in favour
    /// of a hand-grouped `after_help` string, so a new subcommand
    /// added to the enum is invisible to users until it is also
    /// listed in `after_help`. This test catches that omission.
    ///
    /// If this test fails:
    ///   1. Open crates/joy-cli/src/main.rs.
    ///   2. Find the `after_help = "\` block on the top-level `Cli`
    ///      `#[command(...)]` attribute.
    ///   3. Add a line for the missing subcommand to the group it
    ///      belongs to (Core Commands / Shortcuts / Discovery &
    ///      Reporting / Confidentiality / Project & Members /
    ///      Maintenance), using the same `<name>  <about>` layout as
    ///      its neighbours. Align the description column with two
    ///      spaces past the longest name in that group.
    ///   4. If the new subcommand is genuinely internal (Git driver,
    ///      default no-arg behaviour, etc.) mark it with
    ///      `#[command(hide = true)]` instead, and this test will
    ///      skip it.
    #[test]
    fn after_help_lists_every_visible_subcommand() {
        let cmd = <Cli as clap::CommandFactory>::command();
        let after_help = cmd
            .get_after_help()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let mut missing: Vec<String> = Vec::new();
        for sub in cmd.get_subcommands() {
            if sub.is_hide_set() {
                continue;
            }
            let name = sub.get_name();
            if !after_help.contains(name) {
                missing.push(name.to_string());
            }
        }
        assert!(
            missing.is_empty(),
            "joy --help is missing these subcommands in its grouped \
             after_help block: {missing:?}. See the comment on this \
             test for the exact fix."
        );
    }
}
