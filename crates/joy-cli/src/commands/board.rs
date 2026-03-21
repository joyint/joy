// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;

use joy_core::event_log;
use joy_core::items;
use joy_core::model::item::{Item, Status};
use joy_core::store;

use crate::color;

const STATUS_ORDER: &[(Status, &str)] = &[
    (Status::New, "NEW"),
    (Status::Open, "OPEN"),
    (Status::InProgress, "IN-PROGRESS"),
    (Status::Review, "REVIEW"),
    (Status::Closed, "CLOSED"),
    (Status::Deferred, "DEFERRED"),
];

const THIN_COL_WIDTH: usize = 1;
const MIN_COL_WIDTH: usize = 12;
const COL_GAP: usize = 1;

pub fn run(args: crate::BoardArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;

    let root = match store::find_project_root(&cwd) {
        Some(r) => r,
        None => {
            println!(
                "joy v{} -- run `joy init` to get started",
                env!("CARGO_PKG_VERSION")
            );
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
        let items_in_status: Vec<&Item> =
            all_items.iter().filter(|i| &i.status == status).collect();
        if !items_in_status.is_empty() {
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
    let mut empty_count = 0;

    {
        let mut active_idx = 0;
        for (status, _) in STATUS_ORDER {
            if active_idx < columns.len() && &columns[active_idx].status == status {
                layout.push(ColLayout::Active(active_idx));
                active_idx += 1;
            } else {
                layout.push(ColLayout::Empty);
                empty_count += 1;
            }
        }
    }

    let thin_total = empty_count * (THIN_COL_WIDTH + COL_GAP);
    let gaps_between_active = active_count.saturating_sub(1);
    let available = term_width
        .saturating_sub(thin_total)
        .saturating_sub(gaps_between_active * COL_GAP);
    let col_width = (available / active_count).max(MIN_COL_WIDTH);

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
            let date = &e.timestamp[..10];
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
    header_parts.push(format!("{total_items} items"));
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
            ColLayout::Empty => {
                col_header.push_str(&color::label("."));
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
                ColLayout::Empty => {
                    line.push(' ');
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
    Empty,
}

#[derive(Clone, Copy)]
enum DisplayMode {
    Wide,
    Medium,
    Narrow,
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
