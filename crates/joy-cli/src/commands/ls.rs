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
    #[arg(long)]
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

impl LsArgs {
    pub fn roadmap() -> Self {
        Self {
            parent: None,
            item_type: None,
            status: None,
            priority: None,
            mine: false,
            milestone: None,
            blocked: false,
            all: false,
            tree: true,
            show: Vec::new(),
            group: "milestone".to_string(),
        }
    }
}

/// Resolve the effective milestone for an item: its own, or inherited from ancestors.
pub fn effective_milestone<'a>(item: &'a Item, all_items: &'a [Item]) -> Option<&'a str> {
    if let Some(ref ms) = item.milestone {
        return Some(ms.as_str());
    }
    let mut visited = std::collections::HashSet::new();
    let mut current_parent = item.parent.as_deref();
    while let Some(pid) = current_parent {
        if !visited.insert(pid) {
            break; // cycle detected
        }
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
    let mut visited = std::collections::HashSet::new();
    let mut current = item.parent.as_deref();
    while let Some(pid) = current {
        if !visited.insert(pid) {
            break; // cycle detected
        }
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
            parent: show.iter().any(|s| s == "parent"),
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
            "parent" => print_tree_by_parent(&filtered),
            other => anyhow::bail!("unknown group: {other} (use: parent, milestone)"),
        }
    } else {
        print_table(&filtered, &all_items, &extras);
    }

    Ok(())
}

/// Detect terminal width: crossterm -> COLUMNS env var -> 80.
fn terminal_width() -> usize {
    #[cfg(feature = "tui")]
    {
        if let Ok((cols, _)) = crossterm::terminal::size() {
            return cols as usize;
        }
    }
    std::env::var("COLUMNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(80)
}

/// Truncate a string to `max_len` display columns, appending "..." if truncated.
fn truncate_title(s: &str, max_len: usize) -> String {
    if display_width(s) <= max_len {
        return s.to_string();
    }
    if max_len <= 3 {
        return ".".repeat(max_len);
    }
    let target = max_len - 3;
    let mut width = 0;
    let mut end = 0;
    for c in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if width + w > target {
            break;
        }
        width += w;
        end += c.len_utf8();
    }
    format!("{}...", &s[..end])
}

fn print_table(items: &[&Item], all_items: &[Item], extras: &ExtraColumns) {
    let term_width = terminal_width();

    // Fixed column widths (including trailing space as separator)
    // ID(10) + sp + TYPE(12) + sp + STATUS(13) + sp + PRIORITY(10) + sp + TITLE
    // = 4 separators (spaces) + 45 chars of fixed columns = 49 before extras
    let fixed_width: usize = 10
        + 1
        + 12
        + 1
        + 13
        + 1
        + 10
        + 1
        + if extras.parent { 10 + 1 } else { 0 }
        + if extras.milestone { 8 + 1 } else { 0 }
        + if extras.assignee { 24 + 1 } else { 0 };

    let min_title_width = 20;
    let title_width = if term_width > fixed_width {
        (term_width - fixed_width).max(min_title_width)
    } else {
        min_title_width
    };

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

    let sep_len = term_width.min(fixed_width + title_width);
    println!("{}", color::label(&"-".repeat(sep_len)));

    for item in items {
        let blocked_str = if item.is_blocked_by(all_items) {
            " [blocked]".to_string()
        } else {
            String::new()
        };

        // Build the raw title with blocked suffix, then truncate
        let raw_title = format!("{}{}", item.title, blocked_str);
        let display_title = truncate_title(&raw_title, title_width);

        // Re-apply color to blocked suffix if present and not truncated
        let colored_title = if !blocked_str.is_empty() && display_title.ends_with("[blocked]") {
            let prefix_len = display_title.len() - "[blocked]".len();
            let prefix = &display_title[..prefix_len];
            format!("{}{}", prefix, color::blocked("[blocked]"))
        } else {
            display_title
        };

        let id_str = color::id(&item.id);
        let type_emoji = color::item_type_indicator(&item.item_type);
        let type_str = format!("{}{}", type_emoji, color::item_type(&item.item_type));
        let status_emoji = color::status_indicator(&item.status);
        let status_str = format!("{}{}", status_emoji, color::status(&item.status));
        let priority_str = color::priority(&item.priority);

        let type_raw = format!("{}{}", type_emoji, item.item_type);
        let status_raw = format!("{}{}", status_emoji, item.status);

        let mut line = format!(
            "{} {} {} {}",
            pad_colored(&id_str, &item.id, 10),
            pad_colored(&type_str, &type_raw, 12),
            pad_colored(&status_str, &status_raw, 13),
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

        line.push_str(&format!(" {}", colored_title));
        println!("{line}");
    }

    println!("\n{}", color::label(&format!("{} item(s)", items.len())));
}

fn display_width(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

fn pad_colored(colored: &str, raw: &str, width: usize) -> String {
    let padding = width.saturating_sub(display_width(raw));
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

    let type_emoji = color::item_type_indicator(&item.item_type);
    let status_emoji = color::status_indicator(&item.status);
    println!(
        "{}{} {} [{}{}] [{}{}]",
        tree_chrome,
        color::id(&item.id),
        item.title,
        type_emoji,
        color::item_type(&item.item_type),
        status_emoji,
        color::status(&item.status)
    );

    for (ci, child) in children.iter().enumerate() {
        let child_is_last = ci == children.len() - 1;
        print_tree_node(child, all_items, &child_prefix, child_is_last);
    }
}

// -- Tree by milestone (with parent sub-grouping) --

/// Count all items (including closed) belonging to a milestone via effective_milestone.
fn milestone_counts(ms_id: &str, all_items: &[Item]) -> (usize, usize) {
    let linked: Vec<_> = all_items
        .iter()
        .filter(|i| effective_milestone(i, all_items) == Some(ms_id))
        .collect();
    let closed = linked.iter().filter(|i| !i.is_active()).count();
    (closed, linked.len())
}

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
        let children = groups.remove(&ms.id);
        let (closed, total) = milestone_counts(&ms.id, all_items);
        if total == 0 {
            continue;
        }
        if !first {
            println!();
        }
        first = false;
        let empty = vec![];
        print_milestone_group(
            &ms.id,
            Some(&ms.title),
            ms.date.as_ref(),
            closed,
            total,
            children.as_deref().unwrap_or(&empty),
        );
    }

    // Print unknown milestone IDs (referenced but no .yaml file)
    for (ms_id, children) in &groups {
        if !known_ids.contains(ms_id.as_str()) {
            if !first {
                println!();
            }
            first = false;
            let (closed, total) = milestone_counts(ms_id, all_items);
            print_milestone_group(ms_id, None, None, closed, total, children);
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
    closed: usize,
    total: usize,
    children: &[&&Item],
) {
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

    let type_emoji = color::item_type_indicator(&item.item_type);
    let status_emoji = color::status_indicator(&item.status);
    println!(
        "{}{} {} [{}{}] [{}{}]",
        tree_chrome,
        color::id(&item.id),
        item.title,
        type_emoji,
        color::item_type(&item.item_type),
        status_emoji,
        color::status(&item.status)
    );

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
