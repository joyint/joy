// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

mod color;
mod commands;
mod complete;
mod forge;
mod prompt;
mod version_bump;

use std::io::IsTerminal;

use clap::{CommandFactory, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "joy",
    version,
    infer_subcommands = true,
    about = "Terminal-native product management",
    after_help = "\
Quick start:
  joy init                              Set up a new project
  joy add task \"Fix login bug\"          Create an item
  joy ls                                List all items
  joy start IT-0001                     Start working on it
  joy                                   Show the board

Run 'joy tutorial' for the full guide."
)]
pub(crate) struct Cli {
    /// Show all items on the board (no limit per column)
    #[arg(short, long)]
    all: bool,

    /// Reverse sort order
    #[arg(short, long)]
    reverse: bool,

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
    Board(BoardArgs),
    /// Show or modify configuration
    Config(commands::config::ConfigArgs),
    /// AI tool integration
    Ai(commands::ai::AiArgs),
    /// Authenticate (enter passphrase to start a session)
    Auth(commands::auth::AuthArgs),
    /// End the current session
    Deauth(commands::deauth::DeauthArgs),
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

fn main() -> anyhow::Result<()> {
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();

    let raw: Vec<String> = std::env::args().collect();
    let cli = Cli::parse_from(rewrite_trailing_help(raw, &Cli::command()));

    // Config subcommand handles its own validation, run it before load_config
    // to avoid duplicate warnings for invalid config state.
    if let Some(Commands::Config(args)) = cli.command {
        return commands::config::run(args);
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
        None => commands::board::run(BoardArgs {
            filter: commands::filter_args::FilterArgs::default(),
            short: false,
            all: cli.all,
            reverse: cli.reverse,
        }),
    };

    if show_fortune && result.is_ok() && config.output.fortune && std::io::stdout().is_terminal() {
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
}
