// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::{bail, Result};
use clap::Args;

use joy_core::guard::Action;
use joy_core::items;
use joy_core::model::item::{Capability, ItemType, Priority, Status};
use joy_core::store;
use joy_core::templates;

#[derive(Args)]
#[command(
    override_usage = "joy add [TYPE] [TITLE] [SCOPE] [OPTIONS]",
    after_help = "\
Item IDs use the project acronym as prefix and are auto-generated:
  ACRONYM-0001 to ACRONYM-FFFF (e.g. JOY-0001, JOY-00AF)

Use --id to assign a specific ID manually."
)]
pub struct AddArgs {
    /// Item type: epic|story|task|bug|rework|decision|idea|job
    #[arg(index = 1, value_name = "TYPE")]
    pos_type: Option<String>,

    /// Item title
    #[arg(index = 2, value_name = "TITLE")]
    pos_title: Option<String>,

    /// Scope item IDs for a job (comma-separated)
    #[arg(index = 3, value_name = "SCOPE")]
    pos_scope: Option<String>,

    /// Item title (alternative to positional)
    #[arg(short, long, hide = true)]
    title: Option<String>,

    /// Scope item IDs (alternative to positional; job only)
    #[arg(long, hide = true)]
    scope: Option<String>,

    /// Item type (alt to positional): epic|story|task|bug|rework|decision|idea|job
    #[arg(short = 'T', long = "type", hide = true)]
    item_type: Option<String>,

    /// Priority: low|medium|high|critical|extreme
    #[arg(short, long, default_value = "medium")]
    priority: String,

    /// Parent item ID (epic, story, or task)
    #[arg(long)]
    parent: Option<String>,

    /// Effort: 1-7 or xxs|xs|s|m|l|xl|xxl
    #[arg(short, long)]
    effort: Option<String>,

    /// Description
    #[arg(short, long)]
    description: Option<String>,

    /// Milestone ID
    #[arg(short, long)]
    milestone: Option<String>,

    /// Tags (comma-separated)
    #[arg(long)]
    tags: Option<String>,

    /// Explicit item ID (skip auto-generation)
    #[arg(long)]
    id: Option<String>,

    /// Dependencies (comma-separated IDs)
    #[arg(long)]
    deps: Option<String>,

    /// Status: new|open|in-progress|review|closed|deferred
    #[arg(short, long)]
    status: Option<String>,

    /// Version tag (e.g. v0.5.0)
    #[arg(short = 'v', long)]
    version: Option<String>,

    /// Capabilities (CSV; overrides type defaults)
    #[arg(short = 'c', long)]
    capabilities: Option<String>,

    /// Skip the duplicate-title check (rare; allows two items with the same title).
    #[arg(long)]
    allow_duplicate: bool,
}

pub fn run(args: AddArgs) -> Result<()> {
    // Show help when called without any arguments
    if args.pos_type.is_none()
        && args.pos_title.is_none()
        && args.title.is_none()
        && args.item_type.is_none()
    {
        use clap::CommandFactory;
        // Build a throwaway Cli just to extract the add subcommand help
        let mut cmd = crate::Cli::command();
        let sub = cmd.find_subcommand_mut("add").unwrap();
        sub.print_help()?;
        std::process::exit(0);
    }

    let ctx = crate::crypt_session::load_context(None)?;

    let type_str = args
        .item_type
        .or(args.pos_type)
        .ok_or_else(|| anyhow::anyhow!("type is required: joy add <TYPE> <TITLE> or --type"))?;

    let title = args
        .title
        .or(args.pos_title)
        .ok_or_else(|| anyhow::anyhow!("title is required: joy add <TYPE> \"<TITLE>\""))?;

    let item_type: ItemType = type_str
        .parse()
        .map_err(|e: String| anyhow::anyhow!("{}", e))?;

    // Scope is the job's defining attribute: required there, meaningless
    // (and rejected) everywhere else.
    let scope_str = args.scope.or(args.pos_scope);
    let scope: Option<Vec<String>> = if item_type == ItemType::Job {
        let spec = scope_str.ok_or_else(|| {
            anyhow::anyhow!("a job needs a scope: joy add job \"Title\" JOY-0001,JOY-0002")
        })?;
        let mut scope: Vec<String> = Vec::new();
        for raw in spec.split(',') {
            let sid = raw.trim();
            if sid.is_empty() {
                continue;
            }
            if items::is_job_id(sid) {
                bail!("a job cannot scope another job; use deps for job ordering");
            }
            // load_item also normalizes short forms to full IDs.
            let scope_item = items::load_item(&ctx.root, sid)
                .map_err(|_| anyhow::anyhow!("scope item {} is not a valid item ID.", sid))?;
            if scope_item.item_type == ItemType::Job {
                bail!("a job cannot scope another job; use deps for job ordering");
            }
            if !scope.contains(&scope_item.id) {
                scope.push(scope_item.id);
            }
        }
        if scope.is_empty() {
            bail!("a job needs a scope: joy add job \"Title\" JOY-0001,JOY-0002");
        }
        Some(scope)
    } else {
        if scope_str.is_some() {
            bail!("scope is only valid for job items");
        }
        None
    };

    if item_type == ItemType::Job && args.milestone.is_some() {
        bail!("a job cannot be linked to a milestone");
    }

    // Refuse to silently create a second item with an identical title;
    // downstream tools (and humans) cannot disambiguate by title once
    // duplicates exist. See JOY-0170-08.
    if !args.allow_duplicate {
        let existing = items::load_items(&ctx.root)?;
        let title_lc = title.trim().to_lowercase();
        let collision: Vec<&joy_core::model::item::Item> = existing
            .iter()
            .filter(|i| i.title.trim().to_lowercase() == title_lc)
            .collect();
        if !collision.is_empty() {
            let ids: Vec<String> = collision.iter().map(|i| i.id.clone()).collect();
            bail!(
                "an item with this title already exists: {}\n  pass --allow-duplicate to create another one anyway",
                ids.join(", ")
            );
        }
    }

    let priority: Priority = args
        .priority
        .parse()
        .map_err(|e: String| anyhow::anyhow!("{}", e))?;

    let id = match args.id {
        Some(id) => {
            if items::find_item_file(&ctx.root, &id).is_ok() {
                bail!("item {} already exists", id);
            }
            id
        }
        None => {
            let acronym = store::load_acronym(&ctx.root)?;
            if item_type == ItemType::Job {
                items::next_job_id(&ctx.root, &acronym, &title)?
            } else {
                items::next_id(&ctx.root, &acronym, &title)?
            }
        }
    };

    let mut item = templates::render_item(&item_type, &id, &title)?;

    if let Some(scope) = scope {
        item.job = Some(joy_core::model::item::JobSpec {
            scope,
            budget: None,
            window: None,
            attempts: vec![],
        });
    }

    item.priority = priority;
    item.parent = args.parent;
    item.description = args.description;
    item.milestone = args.milestone;
    item.tags = args
        .tags
        .map(|t| t.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();
    item.deps = args
        .deps
        .map(|d| d.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    if let Some(ref caps) = args.capabilities {
        item.capabilities = caps
            .split(',')
            .map(|s| {
                s.trim()
                    .parse::<Capability>()
                    .map_err(|e| anyhow::anyhow!("{}", e))
            })
            .collect::<Result<Vec<_>>>()?;
    }

    item.version = args.version;
    if let Some(ref e) = args.effort {
        item.effort = crate::effort::parse_effort(e)?;
    }

    if let Some(ref s) = args.status {
        item.status = s
            .parse::<Status>()
            .map_err(|e: String| anyhow::anyhow!("{}", e))?;
    }

    // Validate parent exists as an item
    if let Some(ref parent_id) = item.parent {
        match items::load_item(&ctx.root, parent_id) {
            Ok(parent) => {
                if !parent.is_active() {
                    eprintln!("Warning: parent {} is {}.", parent_id, parent.status);
                }
            }
            Err(_) => {
                if parent_id.contains("-MS-") {
                    bail!(
                        "{} is a milestone, not an item. Use `joy milestone link <ID> {}` instead.",
                        parent_id,
                        parent_id
                    );
                }
                bail!("parent {} is not a valid item ID.", parent_id);
            }
        }
    }

    ctx.enforce(&Action::CreateItem, &id)?;

    let log_user = ctx.log_user();
    item.created_by = Some(log_user.clone().into());
    item.updated_by = Some(log_user.into());

    items::save_item(&ctx.root, &item)?;
    joy_core::event_log::log_event_as(
        &ctx.root,
        joy_core::event_log::EventType::ItemCreated,
        &id,
        None,
        &ctx.log_user(),
    );

    if crate::output::is_json() {
        crate::output::emit(&item)?;
    } else {
        println!("Created {} {}", id, title);
    }

    joy_core::git_ops::auto_git_post_command(
        &ctx.root,
        &format!("add {id} {title}"),
        &ctx.log_user(),
    );

    Ok(())
}
