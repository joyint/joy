// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

mod color;
mod commands;

use clap::{CommandFactory, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "joy",
    version,
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
}

#[derive(clap::Args)]
pub(crate) struct BoardArgs {
    /// Show all items (no limit per status group)
    #[arg(short, long)]
    pub all: bool,
}

#[derive(clap::Args)]
struct RoadmapArgs {
    /// Show all items (including closed and deferred)
    #[arg(short, long)]
    all: bool,
}

#[derive(clap::Args)]
struct ShortcutArgs {
    /// Item ID (e.g. IT-0001)
    id: String,
}

fn main() -> anyhow::Result<()> {
    let config = joy_core::store::load_config();
    color::init(&config.output);

    let cli = Cli::parse();

    match cli.command {
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
        None => commands::board::run(BoardArgs { all: false }),
    }
}
