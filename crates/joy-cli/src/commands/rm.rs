// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use clap::Args;

use joy_core::identity;
use joy_core::items;
use joy_core::store;

use crate::color;

#[derive(Args)]
pub struct RmArgs {
    /// Item ID (e.g. IT-0001)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    id: String,

    /// Skip confirmation prompt
    #[arg(short, long)]
    force: bool,

    /// Delete item and all its children (recursively)
    #[arg(short, long)]
    recursive: bool,
}

pub fn run(args: RmArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let item = items::load_item(&root, &args.id)?;

    joy_core::guard::enforce(&root, &joy_core::guard::Action::DeleteItem, &item.id, None)?;

    let resolved = identity::resolve_identity(&root).unwrap_or(identity::Identity {
        member: "unknown".into(),
        delegated_by: None,
        authenticated: false,
    });

    let mut to_delete = vec![item.id.clone()];

    if args.recursive {
        let all_items = items::load_items(&root)?;
        collect_descendants(&all_items, &item.id, &mut to_delete);
    }

    if !args.force {
        if args.recursive {
            eprintln!(
                "Delete {} and {}?",
                color::id(&item.id),
                color::plural(to_delete.len() - 1, "child item")
            );
        } else {
            eprintln!(
                "Delete {} {} [{}]?",
                color::id(&item.id),
                item.title,
                color::item_type(&item.item_type)
            );
        }
        eprint!("Confirm (y/N): ");

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    let log_user = resolved.log_user();
    let summary_title = item.title.clone();
    let summary_id = item.id.clone();
    for id in &to_delete {
        let deleted = items::delete_item(&root, id)?;
        let updated = items::remove_references(&root, id)?;
        joy_core::event_log::log_event_as(
            &root,
            joy_core::event_log::EventType::ItemDeleted,
            id,
            Some(&deleted.title),
            &log_user,
        );
        println!("Deleted {} {}", color::id(id), deleted.title);
        for ref_id in &updated {
            println!("  Removed dependency from {}", color::id(ref_id));
        }
    }

    joy_core::git_ops::auto_git_post_command(
        &root,
        &format!("rm {summary_id} {summary_title}"),
        &log_user,
    );

    Ok(())
}

/// Recursively collect all descendant IDs of a given parent.
fn collect_descendants(
    all_items: &[joy_core::model::Item],
    parent_id: &str,
    result: &mut Vec<String>,
) {
    for item in all_items {
        if item.parent.as_deref() == Some(parent_id) && !result.contains(&item.id) {
            result.push(item.id.clone());
            collect_descendants(all_items, &item.id, result);
        }
    }
}
