// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;

use joy_core::event_log;
use joy_core::items;
use joy_core::model::item::{Item, Status};
use joy_core::store;
use joy_core::vcs::Vcs;

use crate::color;
use crate::commands::ai as ai_cmd;
use crate::commands::init::{self as init_cmd, InitArgs};
use crate::prompt;

const STATUS_ORDER: &[(Status, &str)] = &[
    (Status::New, "NEW"),
    (Status::Open, "OPEN"),
    (Status::InProgress, "IN-PROGRESS"),
    (Status::Review, "REVIEW"),
    (Status::Closed, "CLOSED"),
    (Status::Deferred, "DEFERRED"),
];

const MIN_COL_WIDTH: usize = 12;
const COL_GAP: usize = 1;

pub fn run(args: crate::BoardArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;

    let root = match store::find_project_root(&cwd) {
        Some(r) => r,
        None => {
            welcome_and_maybe_init(&cwd)?;
            return Ok(());
        }
    };

    let all_items = items::load_items(&root)?;

    if all_items.is_empty() {
        println!("No items. Run `joy add` to create one.");
        return Ok(());
    }

    let term_width = terminal_width();
    let term_height = terminal_height();

    // Group items by status
    let mut columns: Vec<Column> = Vec::new();
    let mut total_blocked = 0usize;

    for (status, label) in STATUS_ORDER {
        let mut items_in_status: Vec<&Item> =
            all_items.iter().filter(|i| &i.status == status).collect();
        if !items_in_status.is_empty() {
            // Default: newest first; --reverse: oldest first (original ID order)
            if !args.reverse {
                items_in_status.reverse();
            }
            columns.push(Column {
                status: status.clone(),
                label,
                items: items_in_status,
            });
        }
    }

    if columns.is_empty() {
        println!("No items. Run `joy add` to create one.");
        return Ok(());
    }

    for col in &columns {
        total_blocked += col
            .items
            .iter()
            .filter(|i| i.is_blocked_by(&all_items))
            .count();
    }

    // Build column layout: active columns get equal share, empty stati get thin placeholder
    let mut layout: Vec<ColLayout> = Vec::new();
    let active_count = columns.len();
    {
        let mut active_idx = 0;
        for (status, _) in STATUS_ORDER {
            if active_idx < columns.len() && &columns[active_idx].status == status {
                layout.push(ColLayout::Active(active_idx));
                active_idx += 1;
            } else {
                layout.push(ColLayout::Empty(status.clone()));
            }
        }
    }

    // Calculate widths: each slot gets a width, gaps between all slots
    let total_slots = layout.len();
    let total_gaps = if total_slots > 1 {
        (total_slots - 1) * COL_GAP
    } else {
        0
    };
    let thin_total: usize = layout
        .iter()
        .filter_map(|slot| match slot {
            ColLayout::Empty(status) => Some(thin_indicator_width(status)),
            _ => None,
        })
        .sum();
    let available_for_active = term_width.saturating_sub(total_gaps + thin_total);
    let col_width = if active_count > 0 {
        (available_for_active / active_count).max(MIN_COL_WIDTH)
    } else {
        MIN_COL_WIDTH
    };

    // Determine display mode based on column width
    let mode = if col_width >= 30 {
        DisplayMode::Wide
    } else if col_width >= 18 {
        DisplayMode::Medium
    } else {
        DisplayMode::Narrow
    };

    // Header
    let project = store::load_project(&root).ok();
    let project_name = project
        .as_ref()
        .map(|p| p.name.as_str())
        .unwrap_or("(unnamed)");
    let created = project
        .as_ref()
        .map(|p| p.created.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    let total_items = all_items.len();

    let last_change = event_log::read_events(&root, None, None, 1)
        .ok()
        .and_then(|events| events.into_iter().next());
    let last_str = match &last_change {
        Some(e) => {
            let date = e.timestamp.get(..10).unwrap_or(&e.timestamp);
            let user = e.user.split('@').next().unwrap_or(&e.user);
            format!("{} {}", date, user)
        }
        None => String::new(),
    };

    let sep = color::label(&"-".repeat(term_width));
    println!("{sep}");

    let mut header_parts: Vec<String> = vec![project_name.to_string(), format!("since {created}")];
    if !last_str.is_empty() {
        header_parts.push(last_str);
    }
    header_parts.push(color::plural(total_items, "item"));
    println!("{}", color::label(&header_parts.join(" · ")));

    println!("{sep}");

    // Column headers
    let mut col_header = String::new();
    for (i, slot) in layout.iter().enumerate() {
        if i > 0 {
            col_header.push(' ');
        }
        match slot {
            ColLayout::Active(idx) => {
                let col = &columns[*idx];
                let count_str = format!("{} ({})", col.label, col.items.len());
                let status_ind = color::status_indicator(&col.status);
                let heading = format!(
                    "{}{}",
                    status_ind,
                    color::status_heading(&col.status, &count_str)
                );
                let w = display_width(&heading);
                col_header.push_str(&heading);
                if w < col_width {
                    col_header.push_str(&" ".repeat(col_width - w));
                }
            }
            ColLayout::Empty(status) => {
                let indicator = color::status_indicator(status);
                if indicator.is_empty() {
                    // No emoji: use first letter of status
                    let first = match status {
                        Status::New => "N",
                        Status::Open => "O",
                        Status::InProgress => "W",
                        Status::Review => "R",
                        Status::Closed => "C",
                        Status::Deferred => "D",
                    };
                    col_header.push_str(&color::inactive(first));
                } else {
                    col_header.push_str(&color::inactive(indicator.trim()));
                }
            }
        }
    }
    println!("{col_header}");

    // Height: terminal minus chrome (header 3 + col header 1 + footer 2 + prompt space 7)
    let chrome_lines = 13;
    let max_body_lines = if args.all {
        usize::MAX
    } else {
        term_height.saturating_sub(chrome_lines)
    };

    let lines_per_item: usize = match mode {
        DisplayMode::Wide | DisplayMode::Narrow => 1,
        DisplayMode::Medium => 2,
    };

    let max_items_per_col = if lines_per_item > 0 {
        max_body_lines / lines_per_item
    } else {
        0
    };

    // Render each active column
    let rendered: Vec<Vec<String>> = columns
        .iter()
        .map(|col| {
            let show_count = if args.all {
                col.items.len()
            } else {
                col.items.len().min(max_items_per_col)
            };

            let mut lines: Vec<String> = Vec::new();

            for item in col.items.iter().take(show_count) {
                let blocked_suffix = if item.is_blocked_by(&all_items) {
                    format!(" {}", color::blocked("!"))
                } else {
                    String::new()
                };

                match mode {
                    DisplayMode::Wide => {
                        // ID type prio eff title -- compact indicators only
                        let id_colored = color::id(&item.id);
                        let (_, type_colored) = color::item_type_display(&item.item_type);
                        let (_, prio_colored) = color::priority_display(&item.priority);
                        let eff_colored = color::effort_indicator(item.effort);

                        // In board: always use short form (emoji or abbr)
                        let type_short = if color::is_short() {
                            type_colored.clone()
                        } else {
                            // Force short: emoji only or abbr
                            let ind = color::item_type_indicator(&item.item_type);
                            if ind.is_empty() {
                                color::item_type_colored_short(&item.item_type)
                            } else {
                                ind.trim().to_string()
                            }
                        };
                        let prio_short = if color::is_short() {
                            prio_colored.clone()
                        } else {
                            let ind = color::priority_indicator(&item.priority);
                            if ind.is_empty() {
                                color::priority_colored_short(&item.priority)
                            } else {
                                ind.trim().to_string()
                            }
                        };

                        let prefix = format!(
                            "{} {} {} {} ",
                            id_colored, type_short, prio_short, eff_colored
                        );
                        let prefix_w = display_width(&prefix);
                        let title_space =
                            col_width.saturating_sub(prefix_w + display_width(&blocked_suffix));
                        let title = truncate_display(&item.title, title_space);
                        lines.push(format!("{prefix}{title}{blocked_suffix}"));
                    }
                    DisplayMode::Medium => {
                        // Line 1: ID + blocked
                        let id_colored = color::id(&item.id);
                        lines.push(format!("{}{}", id_colored, blocked_suffix));
                        // Line 2: truncated title
                        let title = truncate_display(&item.title, col_width);
                        lines.push(title);
                    }
                    DisplayMode::Narrow => {
                        // Single line: ID only + blocked
                        let id_colored = color::id(&item.id);
                        lines.push(format!("{}{}", id_colored, blocked_suffix));
                    }
                }
            }

            if show_count < col.items.len() {
                let more = col.items.len() - show_count;
                lines.push(color::label(&format!("+{more} more")));
            }

            lines
        })
        .collect();

    // Print rows side by side
    let max_lines = rendered.iter().map(|c| c.len()).max().unwrap_or(0);

    for row in 0..max_lines {
        let mut line = String::new();
        for (i, slot) in layout.iter().enumerate() {
            if i > 0 {
                line.push(' ');
            }
            match slot {
                ColLayout::Active(idx) => {
                    let col_lines = &rendered[*idx];
                    if row < col_lines.len() {
                        let cell = &col_lines[row];
                        let cell_w = display_width(cell);
                        line.push_str(cell);
                        if cell_w < col_width {
                            line.push_str(&" ".repeat(col_width - cell_w));
                        }
                    } else {
                        line.push_str(&" ".repeat(col_width));
                    }
                }
                ColLayout::Empty(status) => {
                    let w = thin_indicator_width(status);
                    line.push_str(&" ".repeat(w));
                }
            }
        }
        println!("{line}");
    }

    // Footer
    println!("{sep}");

    let mut counts: Vec<String> = Vec::new();
    for col in &columns {
        let count = col.items.len();
        let label = col.label.to_lowercase();
        counts.push(format!("{count} {label}"));
    }
    if total_blocked > 0 {
        counts.push(format!("{total_blocked} blocked"));
    }
    println!("{}", color::label(&counts.join(" · ")));

    Ok(())
}

struct Column<'a> {
    status: Status,
    label: &'a str,
    items: Vec<&'a Item>,
}

enum ColLayout {
    Active(usize),
    Empty(Status),
}

#[derive(Clone, Copy)]
enum DisplayMode {
    Wide,
    Medium,
    Narrow,
}

/// Get the display width of a thin (empty) column indicator.
fn thin_indicator_width(status: &Status) -> usize {
    let indicator = color::status_indicator(status);
    if indicator.is_empty() {
        1 // single letter fallback
    } else {
        display_width(indicator.trim())
    }
}

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

fn terminal_height() -> usize {
    #[cfg(feature = "tui")]
    {
        if let Ok((_, rows)) = crossterm::terminal::size() {
            return rows as usize;
        }
    }
    std::env::var("LINES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24)
}

/// Measure the display width of a string, stripping ANSI escape codes.
fn display_width(s: &str) -> usize {
    let stripped = strip_ansi(s);
    unicode_width::UnicodeWidthStr::width(stripped.as_str())
}

/// Truncate a string to fit within `max_cols` display columns, appending "..." if needed.
fn truncate_display(s: &str, max_cols: usize) -> String {
    let w = unicode_width::UnicodeWidthStr::width(s);
    if w <= max_cols {
        return s.to_string();
    }
    if max_cols <= 3 {
        return ".".repeat(max_cols);
    }
    let target = max_cols - 3;
    let mut width = 0;
    let mut end = 0;
    for c in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if width + cw > target {
            break;
        }
        width += cw;
        end += c.len_utf8();
    }
    format!("{}...", &s[..end])
}

/// Shown when `joy` runs outside a project. Prints a short next-steps
/// block and, on an interactive terminal, offers to start a guided
/// init. On a pipe/CI the prompt is skipped.
fn welcome_and_maybe_init(cwd: &std::path::Path) -> Result<()> {
    println!();
    println!(
        "{}",
        color::label(&format!("joy {}", env!("CARGO_PKG_VERSION")))
    );
    println!();
    println!("No joy project here. Next steps:");
    println!("  joy init             Initialize this directory as a project");
    println!("  joy help             List all commands");
    println!("  joy tutorial         Walkthrough");
    println!();
    println!("Docs: https://joyint.com/en/joy/docs");
    println!();

    if !prompt::is_interactive() {
        return Ok(());
    }

    if !prompt::ask_yn("Initialize a project here?", false)? {
        return Ok(());
    }

    let default_name = cwd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project")
        .to_string();
    let name = prompt::ask_text("Name", Some(&default_name))?;
    let acronym_default = derive_acronym(&name);
    let acronym = prompt::ask_text("Acronym", Some(&acronym_default))?;

    let git_email = joy_core::vcs::default_vcs()
        .user_email()
        .ok()
        .filter(|s| !s.is_empty());
    let user = prompt::ask_text("User", git_email.as_deref())?;
    let language = prompt::ask_text("Language (e.g. en, de)", Some("en"))?;

    println!();
    init_cmd::run(InitArgs {
        name: Some(name),
        acronym: Some(acronym),
        user: Some(user),
        language: Some(language),
    })?;

    if prompt::ask_yn("Initialize AI tools now?", false)? {
        println!();
        ai_cmd::run_init_default()?;
    }

    Ok(())
}

/// Derive a short uppercase acronym from a project name. The full
/// acronym logic lives in joy-core and runs again inside init::init()
/// if we pass the name without an acronym. This helper only exists to
/// show the user a realistic default in the prompt.
fn derive_acronym(name: &str) -> String {
    let letters: String = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.chars().next())
        .take(4)
        .collect::<String>()
        .to_uppercase();
    if letters.len() >= 2 {
        letters
    } else {
        name.chars()
            .filter(|c| c.is_ascii_alphabetic())
            .take(3)
            .collect::<String>()
            .to_uppercase()
    }
}

fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_escape = false;
    for c in s.chars() {
        if in_escape {
            if c.is_ascii_alphabetic() {
                in_escape = false;
            }
            continue;
        }
        if c == '\x1b' {
            in_escape = true;
            continue;
        }
        result.push(c);
    }
    result
}
