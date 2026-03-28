// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::Utc;
use clap::Args;

use joy_core::identity;
use joy_core::items;
use joy_core::store;

use crate::color;

#[derive(Args)]
#[command(after_help = "\
Direction: <ID> depends on <ADD/RM>, i.e. ADD/RM must be completed first.

Examples:
  joy deps IT-0002 --add IT-0001   IT-0002 depends on IT-0001
  joy deps IT-0002 --rm IT-0001    Remove that dependency
  joy deps IT-0002                 List dependencies of IT-0002
  joy deps IT-0002 --tree          Show full dependency tree")]
pub struct DepsArgs {
    /// Item ID (e.g. IT-0001)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    id: String,

    /// Add dependency: <ID> will depend on this item
    #[arg(long)]
    add: Option<String>,

    /// Remove dependency: <ID> will no longer depend on this item
    #[arg(long)]
    rm: Option<String>,

    /// Show dependency tree
    #[arg(long)]
    tree: bool,
}

pub fn run(args: DepsArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    if let Some(ref dep_id) = args.add {
        return add_dep(&root, &args.id, dep_id);
    }

    if let Some(ref dep_id) = args.rm {
        return rm_dep(&root, &args.id, dep_id);
    }

    // Show dependencies
    let item = items::load_item(&root, &args.id)?;
    let all_items = items::load_items(&root)?;

    if item.deps.is_empty() {
        println!("{} has no dependencies.", color::id(&item.id));
        return Ok(());
    }

    if args.tree {
        println!("{} {}", color::id(&item.id), item.title);
        print_dep_tree(&all_items, &item.deps, "  ", &mut Vec::new());
    } else {
        println!("Dependencies of {} {}:", color::id(&item.id), item.title);
        for dep_id in &item.deps {
            let info = all_items
                .iter()
                .find(|i| &i.id == dep_id)
                .map(|i| format!("{} [{}]", i.title, color::status(&i.status)))
                .unwrap_or_else(|| "(not found)".to_string());
            println!("  {} {}", color::id(dep_id), info);
        }
    }

    Ok(())
}

fn add_dep(root: &std::path::Path, item_id: &str, dep_id: &str) -> Result<()> {
    joy_core::guard::enforce(root, &joy_core::guard::Action::UpdateItem, item_id, None)?;

    // Verify dep exists
    let _ = items::load_item(root, dep_id)?;

    let mut item = items::load_item(root, item_id)?;

    if item.deps.contains(&dep_id.to_string()) {
        println!(
            "{} already depends on {}",
            color::id(item_id),
            color::id(dep_id)
        );
        return Ok(());
    }

    // Check for cycles
    if let Some(cycle) = items::detect_cycle(root, item_id, dep_id)? {
        let path: Vec<String> = cycle.iter().map(|id| color::id(id)).collect();
        anyhow::bail!("circular dependency detected: {}", path.join(" -> "));
    }

    item.deps.push(dep_id.to_string());
    item.updated = Utc::now();
    items::update_item(root, &item)?;

    let log_user = identity::resolve_identity(root)
        .map(|id| id.log_user())
        .unwrap_or_default();
    joy_core::event_log::log_event_as(
        root,
        joy_core::event_log::EventType::DepAdded,
        item_id,
        Some(dep_id),
        &log_user,
    );

    println!(
        "{} now depends on {}",
        color::id(item_id),
        color::id(dep_id)
    );

    joy_core::git_ops::auto_git_post_command(
        root,
        &format!("deps {item_id} add {dep_id}"),
        &log_user,
    );

    Ok(())
}

fn rm_dep(root: &std::path::Path, item_id: &str, dep_id: &str) -> Result<()> {
    joy_core::guard::enforce(root, &joy_core::guard::Action::UpdateItem, item_id, None)?;

    let mut item = items::load_item(root, item_id)?;

    if !item.deps.contains(&dep_id.to_string()) {
        println!(
            "{} does not depend on {}",
            color::id(item_id),
            color::id(dep_id)
        );
        return Ok(());
    }

    item.deps.retain(|d| d != dep_id);
    item.updated = Utc::now();
    items::update_item(root, &item)?;

    let log_user = identity::resolve_identity(root)
        .map(|id| id.log_user())
        .unwrap_or_default();
    joy_core::event_log::log_event_as(
        root,
        joy_core::event_log::EventType::DepRemoved,
        item_id,
        Some(dep_id),
        &log_user,
    );

    println!(
        "Removed dependency {} from {}",
        color::id(dep_id),
        color::id(item_id)
    );

    joy_core::git_ops::auto_git_post_command(
        root,
        &format!("deps {item_id} rm {dep_id}"),
        &log_user,
    );

    Ok(())
}

fn print_dep_tree(
    all_items: &[joy_core::model::Item],
    deps: &[String],
    indent: &str,
    visited: &mut Vec<String>,
) {
    for (i, dep_id) in deps.iter().enumerate() {
        let is_last = i == deps.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let child_indent = if is_last {
            format!("{indent}    ")
        } else {
            format!("{indent}│   ")
        };

        if visited.contains(dep_id) {
            println!("{indent}{connector}{} (circular)", color::id(dep_id));
            continue;
        }

        if let Some(item) = all_items.iter().find(|i| &i.id == dep_id) {
            println!(
                "{indent}{connector}{} {} [{}]",
                color::id(dep_id),
                item.title,
                color::status(&item.status)
            );
            if !item.deps.is_empty() {
                visited.push(dep_id.clone());
                print_dep_tree(all_items, &item.deps, &child_indent, visited);
                visited.pop();
            }
        } else {
            println!("{indent}{connector}{} (not found)", color::id(dep_id));
        }
    }
}
