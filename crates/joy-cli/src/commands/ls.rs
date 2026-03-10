// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use clap::Args;

use joy_core::items;
use joy_core::milestones;
use joy_core::model::item::{Item, ItemType, Priority, Status};
use joy_core::store;

use crate::color;

#[derive(Args)]
pub struct LsArgs {
    /// Filter by ancestor item ID (shows the item and all descendants)
    #[arg(long, alias = "epic")]
    parent: Option<String>,

    /// Filter by type: epic, story, task, bug, rework, decision, idea
    #[arg(short = 'T', long = "type")]
    item_type: Option<String>,

    /// Filter by status: new, open, in-progress, review, closed, deferred
    #[arg(long)]
    status: Option<String>,

    /// Filter by priority: low, medium, high, critical
    #[arg(long)]
    priority: Option<String>,

    /// Show only items assigned to me (git config user.email)
    #[arg(long)]
    mine: bool,

    /// Filter by milestone ID (includes items inheriting from parent)
    #[arg(long)]
    milestone: Option<String>,

    /// Show only blocked items
    #[arg(long)]
    blocked: bool,

    /// Show all items (including closed and deferred)
    #[arg(short, long)]
    all: bool,

    /// Show hierarchical tree view
    #[arg(long)]
    tree: bool,

    /// Extra columns to show (comma-separated: milestone, assignee, parent)
    #[arg(short, long, value_delimiter = ',')]
    show: Vec<String>,

    /// Group tree view by: parent (default), milestone
    #[arg(long, default_value = "parent")]
    group: String,
}

/// Resolve the effective milestone for an item: its own, or inherited from ancestors.
fn effective_milestone<'a>(item: &'a Item, all_items: &'a [Item]) -> Option<&'a str> {
    if let Some(ref ms) = item.milestone {
        return Some(ms.as_str());
    }
    // Walk up the parent chain
    let mut current_parent = item.parent.as_deref();
    while let Some(pid) = current_parent {
        if let Some(parent) = all_items.iter().find(|i| i.id == pid) {
            if let Some(ref ms) = parent.milestone {
                return Some(ms.as_str());
            }
            current_parent = parent.parent.as_deref();
        } else {
            break;
        }
    }
    None
}

/// Check if an item is a descendant of the given ancestor.
fn is_descendant(item: &Item, ancestor_id: &str, all_items: &[Item]) -> bool {
    let mut current = item.parent.as_deref();
    while let Some(pid) = current {
        if pid == ancestor_id {
            return true;
        }
        current = all_items
            .iter()
            .find(|i| i.id == pid)
            .and_then(|i| i.parent.as_deref());
    }
    false
}

/// Which extra columns to display in table mode.
struct ExtraColumns {
    milestone: bool,
    assignee: bool,
    parent: bool,
}

impl ExtraColumns {
    fn from_args(show: &[String]) -> Self {
        Self {
            milestone: show.iter().any(|s| s == "milestone" || s == "ms"),
            assignee: show.iter().any(|s| s == "assignee"),
            parent: show.iter().any(|s| s == "parent" || s == "epic"),
        }
    }
}

pub fn run(args: LsArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let all_items = items::load_items(&root)?;

    if all_items.is_empty() {
        println!("No items. Run `joy add` to create one.");
        return Ok(());
    }

    let my_email = if args.mine {
        Some(get_git_email()?)
    } else {
        None
    };

    let type_filter: Option<ItemType> = args
        .item_type
        .as_deref()
        .map(|t| t.parse().map_err(|e: String| anyhow::anyhow!("{}", e)))
        .transpose()?;

    let status_filter: Option<Status> = args
        .status
        .as_deref()
        .map(|s| s.parse().map_err(|e: String| anyhow::anyhow!("{}", e)))
        .transpose()?;

    let priority_filter: Option<Priority> = args
        .priority
        .as_deref()
        .map(|p| p.parse().map_err(|e: String| anyhow::anyhow!("{}", e)))
        .transpose()?;

    let filtered: Vec<&Item> = all_items
        .iter()
        .filter(|item| {
            // By default, exclude closed and deferred
            if !args.all
                && args.status.is_none()
                && matches!(item.status, Status::Closed | Status::Deferred)
            {
                return false;
            }

            if let Some(ref parent_id) = args.parent {
                if item.id != *parent_id && !is_descendant(item, parent_id, &all_items) {
                    return false;
                }
            }

            if let Some(ref t) = type_filter {
                if &item.item_type != t {
                    return false;
                }
            }

            if let Some(ref s) = status_filter {
                if &item.status != s {
                    return false;
                }
            }

            if let Some(ref p) = priority_filter {
                if &item.priority != p {
                    return false;
                }
            }

            if let Some(ref ms) = args.milestone {
                if effective_milestone(item, &all_items) != Some(ms.as_str()) {
                    return false;
                }
            }

            if let Some(ref email) = my_email {
                if item.assignee.as_deref() != Some(email.as_str()) {
                    return false;
                }
            }

            if args.blocked && !item.is_blocked_by(&all_items) {
                return false;
            }

            true
        })
        .collect();

    if filtered.is_empty() {
        println!("No matching items.");
        return Ok(());
    }

    let extras = ExtraColumns::from_args(&args.show);

    if args.tree {
        match args.group.as_str() {
            "milestone" | "ms" => {
                let ms_list = milestones::load_milestones(&root)?;
                print_tree_by_milestone(&filtered, &ms_list, &all_items);
            }
            "parent" | "epic" => print_tree_by_parent(&filtered),
            other => anyhow::bail!("unknown group: {other} (use: parent, milestone)"),
        }
    } else {
        print_table(&filtered, &all_items, &extras);
    }

    Ok(())
}

fn print_table(items: &[&Item], all_items: &[Item], extras: &ExtraColumns) {
    // Build header
    let mut header = format!(
        "{:<10} {:<12} {:<13} {:<10}",
        color::heading("ID"),
        color::heading("TYPE"),
        color::heading("STATUS"),
        color::heading("PRIORITY"),
    );
    if extras.parent {
        header.push_str(&format!(" {:<10}", color::heading("PARENT")));
    }
    if extras.milestone {
        header.push_str(&format!(" {:<8}", color::heading("MS")));
    }
    if extras.assignee {
        header.push_str(&format!(" {:<24}", color::heading("ASSIGNEE")));
    }
    header.push_str(&format!(" {}", color::heading("TITLE")));
    println!("{header}");

    let sep_len = 70
        + if extras.parent { 11 } else { 0 }
        + if extras.milestone { 9 } else { 0 }
        + if extras.assignee { 25 } else { 0 };
    println!("{}", color::label(&"-".repeat(sep_len)));

    for item in items {
        let blocked_str = if item.is_blocked_by(all_items) {
            format!(" {}", color::blocked("[blocked]"))
        } else {
            String::new()
        };

        let id_str = color::id(&item.id);
        let type_str = color::item_type(&item.item_type);
        let status_str = color::status(&item.status);
        let priority_str = color::priority(&item.priority);

        let mut line = format!(
            "{} {} {} {}",
            pad_colored(&id_str, &item.id, 10),
            pad_colored(&type_str, &item.item_type.to_string(), 12),
            pad_colored(&status_str, &item.status.to_string(), 13),
            pad_colored(&priority_str, &item.priority.to_string(), 10),
        );

        if extras.parent {
            let parent_val = item.parent.as_deref().unwrap_or("-");
            line.push_str(&format!(
                " {}",
                pad_colored(&color::id(parent_val), parent_val, 10)
            ));
        }
        if extras.milestone {
            let (ms_val, inherited) = match item.milestone.as_deref() {
                Some(ms) => (ms, false),
                None => match effective_milestone(item, all_items) {
                    Some(ms) => (ms, true),
                    None => ("-", false),
                },
            };
            let display = if inherited {
                format!("{ms_val}*")
            } else {
                ms_val.to_string()
            };
            line.push_str(&format!(
                " {}",
                pad_colored(&color::id(&display), &display, 8)
            ));
        }
        if extras.assignee {
            let assignee_val = item.assignee.as_deref().unwrap_or("-");
            let truncated = if assignee_val.len() > 24 {
                format!("{}...", &assignee_val[..21])
            } else {
                assignee_val.to_string()
            };
            line.push_str(&format!(" {:<24}", truncated));
        }

        line.push_str(&format!(" {}{}", item.title, blocked_str));
        println!("{line}");
    }

    println!("\n{}", color::label(&format!("{} item(s)", items.len())));
}

fn pad_colored(colored: &str, raw: &str, width: usize) -> String {
    let padding = width.saturating_sub(raw.len());
    format!("{}{}", colored, " ".repeat(padding))
}

// -- Tree by parent hierarchy (recursive) --

fn print_tree_by_parent(items: &[&Item]) {
    let item_ids: std::collections::HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();

    // Root items: no parent, or parent not in the filtered set
    let roots: Vec<&&Item> = items
        .iter()
        .filter(|i| match i.parent.as_deref() {
            None => true,
            Some(pid) => !item_ids.contains(pid),
        })
        .collect();

    for (i, root) in roots.iter().enumerate() {
        let is_last = i == roots.len() - 1;
        print_tree_node(root, items, "", is_last);
    }

    println!("\n{}", color::label(&format!("{} item(s)", items.len())));
}

fn print_tree_node(item: &Item, all_items: &[&Item], prefix: &str, is_last: bool) {
    let connector = if prefix.is_empty() {
        String::new() // Root level: no connector
    } else if is_last {
        "└── ".to_string()
    } else {
        "├── ".to_string()
    };

    let child_prefix = if prefix.is_empty() {
        "  ".to_string()
    } else if is_last {
        format!("{prefix}    ")
    } else {
        format!("{prefix}│   ")
    };

    let tree_chrome = color::label(&format!("{prefix}{connector}"));

    // Find children in the filtered set
    let children: Vec<&&Item> = all_items
        .iter()
        .filter(|i| i.parent.as_deref() == Some(&item.id))
        .collect();

    let has_children = !children.is_empty();

    if has_children {
        // Parent items: show like epics (no type tag)
        println!(
            "{}{} {} [{}]",
            tree_chrome,
            color::id(&item.id),
            item.title,
            color::status(&item.status)
        );
    } else {
        // Leaf items: show type tag
        println!(
            "{}{} {} [{}] [{}]",
            tree_chrome,
            color::id(&item.id),
            item.title,
            color::item_type(&item.item_type),
            color::status(&item.status)
        );
    }

    for (ci, child) in children.iter().enumerate() {
        let child_is_last = ci == children.len() - 1;
        print_tree_node(child, all_items, &child_prefix, child_is_last);
    }
}

// -- Tree by milestone (with parent sub-grouping) --

fn print_tree_by_milestone(
    items: &[&Item],
    ms_list: &[joy_core::model::Milestone],
    all_items: &[Item],
) {
    use std::collections::{BTreeMap, HashSet};

    // Collect all milestone IDs using effective milestone (own or inherited from parent)
    let mut groups: BTreeMap<String, Vec<&&Item>> = BTreeMap::new();
    let mut no_milestone: Vec<&&Item> = Vec::new();

    for item in items {
        match effective_milestone(item, all_items) {
            Some(ms_id) => groups.entry(ms_id.to_string()).or_default().push(item),
            None => no_milestone.push(item),
        }
    }

    let known_ids: HashSet<&str> = ms_list.iter().map(|ms| ms.id.as_str()).collect();
    let mut first = true;

    // Print known milestones first (in their defined order)
    for ms in ms_list {
        if let Some(children) = groups.remove(&ms.id) {
            if !first {
                println!();
            }
            first = false;
            print_milestone_group(&ms.id, Some(&ms.title), ms.date.as_ref(), &children);
        }
    }

    // Print unknown milestone IDs (referenced but no .yaml file)
    for (ms_id, children) in &groups {
        if !known_ids.contains(ms_id.as_str()) {
            if !first {
                println!();
            }
            first = false;
            print_milestone_group(ms_id, None, None, children);
        }
    }

    // Items without milestone
    if !no_milestone.is_empty() {
        if !first {
            println!();
        }
        println!("{}", color::label("(no milestone)"));
        print_parent_grouped_children(&no_milestone);
    }

    println!("\n{}", color::label(&format!("{} item(s)", items.len())));
}

fn print_milestone_group(
    id: &str,
    title: Option<&str>,
    date: Option<&chrono::NaiveDate>,
    children: &[&&Item],
) {
    let closed = children.iter().filter(|i| !i.is_active()).count();
    let total = children.len();
    let date_str = date.map(|d| format!(" ({})", d)).unwrap_or_default();
    let title_str = title.unwrap_or("(undefined)");

    println!(
        "{} {}{} [{}/{}]",
        color::id(id),
        color::heading(title_str),
        color::label(&date_str),
        closed,
        total
    );
    print_parent_grouped_children(children);
}

/// Print children grouped by parent hierarchy within a milestone group.
fn print_parent_grouped_children(items: &[&&Item]) {
    let item_ids: std::collections::HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();

    // Root items within this group: no parent, or parent not in this group
    let roots: Vec<&&&Item> = items
        .iter()
        .filter(|i| match i.parent.as_deref() {
            None => true,
            Some(pid) => !item_ids.contains(pid),
        })
        .collect();

    for (i, root) in roots.iter().enumerate() {
        let is_last = i == roots.len() - 1;
        print_ms_tree_node(root, items, "  ", is_last);
    }
}

fn print_ms_tree_node(item: &Item, group: &[&&Item], prefix: &str, is_last: bool) {
    let connector = if is_last { "└── " } else { "├── " };
    let child_prefix = if is_last {
        format!("{prefix}    ")
    } else {
        format!("{prefix}│   ")
    };

    let tree_chrome = color::label(&format!("{prefix}{connector}"));

    let children: Vec<&&&Item> = group
        .iter()
        .filter(|i| i.parent.as_deref() == Some(&item.id))
        .collect();

    let has_children = !children.is_empty();

    if has_children {
        println!(
            "{}{} {} [{}]",
            tree_chrome,
            color::id(&item.id),
            item.title,
            color::status(&item.status)
        );
    } else {
        println!(
            "{}{} {} [{}] [{}]",
            tree_chrome,
            color::id(&item.id),
            item.title,
            color::item_type(&item.item_type),
            color::status(&item.status)
        );
    }

    for (ci, child) in children.iter().enumerate() {
        let child_is_last = ci == children.len() - 1;
        print_ms_tree_node(child, group, &child_prefix, child_is_last);
    }
}

fn get_git_email() -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["config", "user.email"])
        .output()?;
    let email = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if email.is_empty() {
        anyhow::bail!("git config user.email is not set");
    }
    Ok(email)
}
