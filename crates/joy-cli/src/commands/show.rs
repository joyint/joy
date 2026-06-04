// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::{DateTime, Local};
use clap::Args;

use joy_core::items;
use joy_core::model::item::{ItemType, Validity};
use joy_core::store;

use crate::color;

#[derive(Args)]
pub struct ShowArgs {
    /// Item ID (e.g. IT-0001)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    id: String,

    /// Compact output: emoji-only or abbreviations
    #[arg(short = 'S', long)]
    pub short: bool,

    /// Passphrase for encrypted items.
    #[arg(long)]
    passphrase: Option<String>,
}

pub fn run(args: ShowArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    // ADR-040: install zone keys upfront if the active member has any
    // Crypt wraps. No-op for plain projects.
    crate::crypt_session::ensure_zone_keys(args.passphrase.as_deref())?;

    let item = items::load_item(&root, &args.id)?;
    let all_items = items::load_items(&root)?;

    if crate::output::is_json() {
        return crate::output::emit(&item);
    }

    let w = color::terminal_width();
    println!("{}", color::label(&"-".repeat(w)));
    println!("{} {}", color::id(&item.id), color::label(&item.title));
    println!("{}", color::label(&"-".repeat(w)));
    let (_, type_display) = color::item_type_display(&item.item_type);
    let (_, status_display) = color::status_display(&item.status);
    let (_, priority_display) = color::priority_display(&item.priority);
    println!("{} {}", color::label("Type:    "), type_display);
    println!("{} {}", color::label("Status:  "), status_display);
    println!("{} {}", color::label("Priority:"), priority_display);
    // Decisions always show their validity; an unset value reads as proposed
    // (the decision is still being decided). Other types show it only if set.
    if matches!(item.item_type, ItemType::Decision) {
        let validity = item.validity.unwrap_or(Validity::Proposed);
        println!("{} {}", color::label("Validity:"), validity);
    } else if let Some(validity) = item.validity {
        println!("{} {}", color::label("Validity:"), validity);
    }
    if let Some(ref replaced_by) = item.replaced_by {
        println!(
            "{} {}",
            color::label("Replaced by:"),
            color::id(replaced_by)
        );
    }

    if let Some(ref parent) = item.parent {
        println!("{} {}", color::label("Parent:  "), color::id(parent));
    }
    if !item.assignees.is_empty() {
        if item.assignees.len() == 1 && item.assignees[0].capabilities.is_empty() {
            println!("{} {}", color::label("Assignee:"), item.assignees[0].member);
        } else {
            println!("{}:", color::label("Assignees"));
            for a in &item.assignees {
                if a.capabilities.is_empty() {
                    println!("  {}", a.member);
                } else {
                    let caps: Vec<String> = a.capabilities.iter().map(|c| c.to_string()).collect();
                    println!("  {}  {}", a.member, caps.join(", "));
                }
            }
        }
    }
    if let Some(ref milestone) = item.milestone {
        println!("{} {}", color::label("Milestone:"), color::id(milestone));
    }
    if !item.tags.is_empty() {
        println!("{} {}", color::label("Tags:    "), item.tags.join(", "));
    }
    if let Some(ref version) = item.version {
        println!("{} {}", color::label("Version: "), version);
    }
    if !item.capabilities.is_empty() {
        let caps: Vec<String> = item.capabilities.iter().map(|c| c.to_string()).collect();
        println!("{} {}", color::label("Capabilities:"), caps.join(", "));
    }

    // Show item-level mode override (only if explicitly set on the item)
    if let Some(ref mode) = item.mode {
        // Check if clamped by max-mode of first assignee
        let clamped = item.assignees.first().and_then(|a| {
            let project = joy_core::store::load_project(&root).ok()?;
            let member = project.members.get(&a.member)?;
            match &member.capabilities {
                joy_core::model::project::MemberCapabilities::Specific(map) => {
                    // Find the capability for the current status
                    item.capabilities.iter().find_map(|cap| {
                        let config = map.get(cap)?;
                        let max = config.max_mode?;
                        if mode < &max {
                            Some((max, *mode))
                        } else {
                            None
                        }
                    })
                }
                _ => None,
            }
        });

        if let Some((effective, original)) = clamped {
            println!(
                "{} {} {}",
                color::label("Mode:"),
                effective,
                color::inactive(&format!("[project max, item: {original}]"))
            );
        } else {
            println!("{} {}", color::label("Mode:"), mode);
        }
    }

    if !item.deps.is_empty() {
        println!("\n{}:", color::label("Dependencies"));
        for dep_id in &item.deps {
            let dep_info = all_items
                .iter()
                .find(|i| &i.id == dep_id)
                .map(|i| format!("{} [{}]", i.title, color::status(&i.status)))
                .unwrap_or_else(|| "(not found)".to_string());
            println!("  {} {}", color::id(dep_id), dep_info);
        }
    }

    if item.is_blocked_by(&all_items) {
        let blockers: Vec<_> = all_items
            .iter()
            .filter(|i| item.deps.contains(&i.id) && i.is_active())
            .collect();
        println!("\n  {}", color::blocked("BLOCKED"));
        for blocker in &blockers {
            println!(
                "    {} {} [{}]",
                color::id(&blocker.id),
                blocker.title,
                color::status(&blocker.status)
            );
        }
    }

    if let Some(ref desc) = item.description {
        println!();
        print!("{}", joy_core::tutorial::render_markdown(desc.trim_end()));
    }

    if !item.comments.is_empty() {
        println!("\n{}:", color::label("Comments"));
        for (i, comment) in item.comments.iter().enumerate() {
            if i > 0 {
                println!();
            }
            let local_dt: DateTime<Local> = comment.date.with_timezone(&Local);
            let date_str = local_dt.format("%Y-%m-%d %H:%M").to_string();
            // 1-based index lets users locate a comment for `joy comment edit
            // <ID> <INDEX>` or `joy comment rm <ID> <INDEX>` without counting.
            println!(
                "{} {} [{}]",
                color::label(&format!("[{}]", i + 1)),
                color::label(&date_str),
                color::user(&comment.author),
            );
            // Body indented two spaces so the comment block reads as a
            // visual unit and stays separate from item-level content
            // (description above, footer below). Markdown wrap-width is
            // reduced by the indent so a long line never spills back to
            // column zero on a wrap.
            println!();
            let indent = "  ";
            let inner_width = w.saturating_sub(indent.len());
            let body = joy_core::tutorial::render_markdown_with_width(&comment.text, inner_width);
            for line in body.lines() {
                if line.is_empty() {
                    println!();
                } else {
                    println!("{indent}{line}");
                }
            }
            // Per-comment edit audit. Each entry: `Updated: <date> by
            // <editor>`. Indented to align with the body. Skipped when
            // the comment has never been edited.
            if !comment.edits.is_empty() {
                println!();
                for edit in &comment.edits {
                    let edit_local: DateTime<Local> = edit.date.with_timezone(&Local);
                    let edit_str = edit_local.format("%Y-%m-%d %H:%M").to_string();
                    println!(
                        "  {} {} by {}",
                        color::label("Updated:"),
                        color::label(&edit_str),
                        color::user(&edit.by),
                    );
                }
            }
        }
    }

    // Blank line separates the footer block from the last comment body.
    println!();
    println!("{}", color::label(&"-".repeat(w)));
    let created_date = item.created.format("%Y-%m-%d %H:%M").to_string();
    let created_line = match &item.created_by {
        Some(by) => format!(
            "{} {} by {}",
            color::label("Created:"),
            color::label(&created_date),
            color::user(by),
        ),
        None => format!(
            "{} {}",
            color::label("Created:"),
            color::label(&created_date),
        ),
    };
    println!("{created_line}");
    match &item.history {
        None => {
            // Legacy YAML written before `history` shipped: fall back to a
            // single `Updated:` line when the item has been mutated since
            // creation. New items always have `Some(...)` so they never go
            // through this branch.
            if item.updated > item.created {
                let updated_date = item.updated.format("%Y-%m-%d %H:%M").to_string();
                let updated_line = match &item.updated_by {
                    Some(by) => format!(
                        "{} {} by {}",
                        color::label("Updated:"),
                        color::label(&updated_date),
                        color::user(by),
                    ),
                    None => format!(
                        "{} {}",
                        color::label("Updated:"),
                        color::label(&updated_date),
                    ),
                };
                println!("{updated_line}");
            }
        }
        Some(entries) => {
            for entry in entries {
                let entry_date = entry.date.format("%Y-%m-%d %H:%M").to_string();
                println!(
                    "{} {} by {}",
                    color::label("Updated:"),
                    color::label(&entry_date),
                    color::user(&entry.by),
                );
            }
        }
    }

    Ok(())
}
