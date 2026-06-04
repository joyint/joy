// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::io::IsTerminal;
use std::sync::OnceLock;

use joy_core::model::config::{ColorMode, OutputConfig};
use joy_core::model::item::{ItemType, Priority, Status, Validity};

static ENABLED: OnceLock<bool> = OnceLock::new();
static EMOJI_ENABLED: OnceLock<bool> = OnceLock::new();
static SHORT_MODE: OnceLock<bool> = OnceLock::new();

/// Initialize color and emoji support from config. Call once at startup.
pub fn init(output: &OutputConfig) {
    let color_enabled = match output.color {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => {
            if std::env::var_os("NO_COLOR").is_some() {
                false
            } else {
                std::io::stdout().is_terminal()
            }
        }
    };
    let _ = ENABLED.set(color_enabled);

    let emoji_enabled = if std::env::var_os("JOY_NO_EMOJI").is_some() {
        false
    } else {
        output.emoji
    };
    let _ = EMOJI_ENABLED.set(emoji_enabled);

    let short = std::env::var_os("JOY_SHORT").is_some() || output.short;
    let _ = SHORT_MODE.set(short);
}

fn is_emoji_enabled() -> bool {
    *EMOJI_ENABLED.get_or_init(|| false)
}

/// Whether emoji output is enabled (for use in other modules).
pub fn use_emoji() -> bool {
    is_emoji_enabled()
}

pub fn is_short() -> bool {
    *SHORT_MODE.get_or_init(|| false)
}

pub fn item_type_indicator(t: &ItemType) -> &'static str {
    if !is_emoji_enabled() {
        return "";
    }
    match t {
        ItemType::Epic => "\u{1f381} ",
        ItemType::Story => "\u{1f4d6} ",
        ItemType::Task => "\u{1f527} ",
        ItemType::Bug => "\u{1f41e} ",
        ItemType::Rework => "\u{1f504} ",
        ItemType::Decision => "\u{1f4a1} ",
        ItemType::Idea => "\u{2728} ",
    }
}

pub fn status_indicator(s: &Status) -> &'static str {
    if !is_emoji_enabled() {
        return "";
    }
    match s {
        Status::New => "\u{1f331} ",
        Status::Open => "\u{1f7e2} ",
        Status::InProgress => "\u{1f3c3} ",
        Status::Review => "\u{1f440} ",
        Status::Closed => "\u{2705} ",
        Status::Deferred => "\u{1f4a4} ",
    }
}

fn is_enabled() -> bool {
    *ENABLED.get_or_init(|| {
        // Fallback if init() was never called: use auto behavior.
        if std::env::var_os("NO_COLOR").is_some() {
            return false;
        }
        std::io::stdout().is_terminal()
    })
}

// Semantic color constants mapped to ANSI color codes.
// These map to the terminal's color theme (e.g. omarchy themes),
// so they adapt automatically to any color scheme.
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DANGER: &str = "\x1b[31m"; // ANSI 1 -- errors, bugs, critical
const INFO: &str = "\x1b[36m"; // ANSI 6 -- review, ideas
const WARNING: &str = "\x1b[33m"; // ANSI 3 -- in-progress, medium priority
const PRIMARY: &str = "\x1b[34m"; // ANSI 4 -- open status
const ACCENT: &str = "\x1b[35m"; // ANSI 5 -- epics, user, blocked
const INACTIVE: &str = "\x1b[38;5;8m"; // ANSI 8 -- closed items in tree
const SECONDARY: &str = "\x1b[32m"; // ANSI 2 -- IDs, labels, timestamps
const SUCCESS: &str = "\x1b[38;5;10m"; // ANSI 10 -- closed status

fn wrap(code: &str, text: &str) -> String {
    if is_enabled() {
        format!("{code}{text}{RESET}")
    } else {
        text.to_string()
    }
}

fn wrap2(code1: &str, code2: &str, text: &str) -> String {
    if is_enabled() {
        format!("{code1}{code2}{text}{RESET}")
    } else {
        text.to_string()
    }
}

pub fn id(text: &str) -> String {
    wrap(SECONDARY, text)
}

pub fn status(s: &Status) -> String {
    let text = s.to_string();
    match s {
        Status::New => text,
        Status::Open => wrap(PRIMARY, &text),
        Status::InProgress => wrap(WARNING, &text),
        Status::Review => wrap(INFO, &text),
        Status::Closed => wrap(SUCCESS, &text),
        Status::Deferred => wrap(SECONDARY, &text),
    }
}

pub fn priority_indicator(p: &Priority) -> &'static str {
    if !is_emoji_enabled() {
        return "";
    }
    match p {
        Priority::Low => "\u{1f7e2} ",
        Priority::Medium => "\u{1f7e1} ",
        Priority::High => "\u{1f534} ",
        Priority::Critical => "\u{1f6a8} ",
        Priority::Extreme => "\u{1f525} ",
    }
}

/// Combined indicator + label for item type. In short mode: emoji only or abbreviation.
pub fn item_type_display(t: &ItemType) -> (String, String) {
    if is_short() {
        if is_emoji_enabled() {
            let emoji = item_type_indicator(t).trim();
            (emoji.to_string(), emoji.to_string())
        } else {
            let abbr = item_type_short(t);
            (abbr.to_string(), item_type_colored_short(t))
        }
    } else {
        let raw = format!("{}{}", item_type_indicator(t), t);
        let colored = format!("{}{}", item_type_indicator(t), item_type(t));
        (raw, colored)
    }
}

/// Combined indicator + label for status. In short mode: emoji only or abbreviation.
pub fn status_display(s: &Status) -> (String, String) {
    if is_short() {
        if is_emoji_enabled() {
            let emoji = status_indicator(s).trim();
            (emoji.to_string(), emoji.to_string())
        } else {
            let abbr = status_short(s);
            (abbr.to_string(), status_colored_short(s))
        }
    } else {
        let raw = format!("{}{}", status_indicator(s), s);
        let colored = format!("{}{}", status_indicator(s), status(s));
        (raw, colored)
    }
}

/// Combined indicator + label for priority. In short mode: emoji only or abbreviation.
pub fn priority_display(p: &Priority) -> (String, String) {
    if is_short() {
        if is_emoji_enabled() {
            let emoji = priority_indicator(p).trim();
            (emoji.to_string(), emoji.to_string())
        } else {
            let abbr = priority_short(p);
            (abbr.to_string(), priority_colored_short(p))
        }
    } else {
        let raw = format!("{}{}", priority_indicator(p), p);
        let colored = format!("{}{}", priority_indicator(p), priority(p));
        (raw, colored)
    }
}

pub fn item_type_colored_short(t: &ItemType) -> String {
    let text = item_type_short(t);
    match t {
        ItemType::Epic => wrap(ACCENT, text),
        ItemType::Story => wrap(PRIMARY, text),
        ItemType::Bug => wrap(DANGER, text),
        ItemType::Rework => wrap(WARNING, text),
        ItemType::Idea => wrap(INFO, text),
        ItemType::Decision => wrap(INFO, text),
        ItemType::Task => wrap(SECONDARY, text),
    }
}

fn status_colored_short(s: &Status) -> String {
    let text = status_short(s);
    match s {
        Status::New => text.to_string(),
        Status::Open => wrap(PRIMARY, text),
        Status::InProgress => wrap(WARNING, text),
        Status::Review => wrap(INFO, text),
        Status::Closed => wrap(SUCCESS, text),
        Status::Deferred => wrap(SECONDARY, text),
    }
}

pub fn priority_colored_short(p: &Priority) -> String {
    let text = priority_short(p);
    match p {
        Priority::Extreme => wrap2(BOLD, DANGER, text),
        Priority::Critical => wrap2(BOLD, DANGER, text),
        Priority::High => wrap(DANGER, text),
        Priority::Medium => wrap(WARNING, text),
        Priority::Low => text.to_string(),
    }
}

pub fn item_type_short(t: &ItemType) -> &'static str {
    match t {
        ItemType::Epic => "epc",
        ItemType::Story => "str",
        ItemType::Task => "tsk",
        ItemType::Bug => "bug",
        ItemType::Rework => "rwk",
        ItemType::Decision => "dec",
        ItemType::Idea => "ide",
    }
}

pub fn status_short(s: &Status) -> &'static str {
    match s {
        Status::New => "new",
        Status::Open => "opn",
        Status::InProgress => "wip",
        Status::Review => "rev",
        Status::Closed => "don",
        Status::Deferred => "def",
    }
}

pub fn priority_short(p: &Priority) -> &'static str {
    match p {
        Priority::Low => "low",
        Priority::Medium => "med",
        Priority::High => "hig",
        Priority::Critical => "crt",
        Priority::Extreme => "ext",
    }
}

pub fn priority(p: &Priority) -> String {
    let text = p.to_string();
    match p {
        Priority::Extreme => wrap2(BOLD, DANGER, &text),
        Priority::Critical => wrap2(BOLD, DANGER, &text),
        Priority::High => wrap(DANGER, &text),
        Priority::Medium => wrap(WARNING, &text),
        Priority::Low => text,
    }
}

pub fn item_type(t: &ItemType) -> String {
    let text = t.to_string();
    match t {
        ItemType::Epic => wrap(ACCENT, &text),
        ItemType::Story => wrap(PRIMARY, &text),
        ItemType::Bug => wrap(DANGER, &text),
        ItemType::Rework => wrap(WARNING, &text),
        ItemType::Idea => wrap(INFO, &text),
        ItemType::Decision => wrap(INFO, &text),
        ItemType::Task => wrap(SECONDARY, &text),
    }
}

pub fn validity_indicator(v: &Validity) -> &'static str {
    if !is_emoji_enabled() {
        return "";
    }
    match v {
        Validity::Proposed => "\u{1f4ad} ",
        Validity::Accepted => "\u{1f4dc} ",
        Validity::Rejected => "\u{1f6ab} ",
        Validity::Replaced => "\u{1f500} ",
        Validity::Retired => "\u{1f4e6} ",
    }
}

pub fn validity_short(v: &Validity) -> &'static str {
    match v {
        Validity::Proposed => "pro",
        Validity::Accepted => "acc",
        Validity::Rejected => "rej",
        Validity::Replaced => "rep",
        Validity::Retired => "ret",
    }
}

pub fn validity(v: &Validity) -> String {
    let text = v.to_string();
    match v {
        Validity::Accepted => wrap(SUCCESS, &text),
        Validity::Proposed => wrap(WARNING, &text),
        Validity::Rejected => wrap(DANGER, &text),
        Validity::Replaced => wrap(INACTIVE, &text),
        Validity::Retired => wrap(INACTIVE, &text),
    }
}

pub fn validity_colored_short(v: &Validity) -> String {
    let text = validity_short(v);
    match v {
        Validity::Accepted => wrap(SUCCESS, text),
        Validity::Proposed => wrap(WARNING, text),
        Validity::Rejected => wrap(DANGER, text),
        Validity::Replaced => wrap(INACTIVE, text),
        Validity::Retired => wrap(INACTIVE, text),
    }
}

/// Combined indicator + label for validity. In short mode: emoji only or abbreviation.
pub fn validity_display(v: &Validity) -> (String, String) {
    if is_short() {
        if is_emoji_enabled() {
            let emoji = validity_indicator(v).trim();
            (emoji.to_string(), emoji.to_string())
        } else {
            let abbr = validity_short(v);
            (abbr.to_string(), validity_colored_short(v))
        }
    } else {
        let raw = format!("{}{}", validity_indicator(v), v);
        let colored = format!("{}{}", validity_indicator(v), validity(v));
        (raw, colored)
    }
}

pub fn user(text: &str) -> String {
    wrap(ACCENT, text)
}

pub fn blocked(text: &str) -> String {
    wrap(ACCENT, text)
}

pub fn label(text: &str) -> String {
    wrap(SECONDARY, text)
}

pub fn inactive(text: &str) -> String {
    wrap(INACTIVE, text)
}

#[allow(dead_code)]
pub fn heading(text: &str) -> String {
    wrap(BOLD, text)
}

pub fn status_heading(s: &Status, text: &str) -> String {
    match s {
        Status::New => wrap(BOLD, text),
        Status::Open => wrap2(BOLD, PRIMARY, text),
        Status::InProgress => wrap2(BOLD, WARNING, text),
        Status::Review => wrap2(BOLD, INFO, text),
        Status::Closed => wrap2(BOLD, SUCCESS, text),
        Status::Deferred => wrap(SECONDARY, text),
    }
}

/// Raw text label for an effort value, used for column-width calculation.
/// - emoji on, short on:  block character only (e.g. "▇")
/// - emoji on, short off: block character + space + t-shirt size (e.g. "▇ xxl")
/// - emoji off:           t-shirt size only (e.g. "xxl")
pub fn effort_label(effort: Option<u8>) -> String {
    let blocks = ['▁', '▂', '▃', '▄', '▅', '▆', '▇'];
    let tshirt = ["xxs", "xs", "s", "m", "l", "xl", "xxl"];
    match effort {
        Some(s) if (1..=7).contains(&s) => {
            let idx = (s - 1) as usize;
            if is_emoji_enabled() {
                if is_short() {
                    blocks[idx].to_string()
                } else {
                    format!("{} {}", blocks[idx], tshirt[idx])
                }
            } else {
                tshirt[idx].to_string()
            }
        }
        _ => " ".to_string(),
    }
}

/// Format an effort value (1-7) for terminal display, coloured per scale.
/// Layout follows effort_label.
/// Colors: 1-2 green, 3-4 yellow, 5 orange, 6-7 red.
pub fn effort_indicator(effort: Option<u8>) -> String {
    const GREEN: &str = "\x1b[32m";
    const YELLOW: &str = "\x1b[33m";
    const ORANGE: &str = "\x1b[38;5;208m";
    const RED: &str = "\x1b[31m";
    const RESET: &str = "\x1b[0m";

    match effort {
        Some(s) if (1..=7).contains(&s) => {
            let label = effort_label(Some(s));
            let color = match s {
                1 | 2 => GREEN,
                3 | 4 => YELLOW,
                5 => ORANGE,
                6 | 7 => RED,
                _ => "",
            };
            if is_color_enabled() {
                format!("{color}{label}{RESET}")
            } else {
                label
            }
        }
        _ => " ".to_string(),
    }
}

fn is_color_enabled() -> bool {
    *ENABLED.get().unwrap_or(&false)
}

// ---------------------------------------------------------------------------
// Layout helpers -- shared across all commands for consistent output
// ---------------------------------------------------------------------------

/// Detect terminal width, falling back to 80 columns.
pub fn terminal_width() -> usize {
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

/// Full-width header: separator, title, separator (matches joy ls).
pub fn header(title: &str) -> String {
    let w = terminal_width();
    let line = label(&"-".repeat(w));
    format!("{}\n{}\n{}", line, label(title), line)
}

/// Full-width separator line.
#[allow(dead_code)]
pub fn separator() -> String {
    label(&"-".repeat(terminal_width()))
}

/// Full-width footer: separator line with a summary message.
pub fn footer(message: &str) -> String {
    let w = terminal_width();
    format!("{}\n{}", label(&"-".repeat(w)), label(message))
}

/// Section heading: secondary-colored with a short underline.
pub fn section(title: &str) -> String {
    let underline = "-".repeat(title.len());
    format!("{}\n{}", label(title), inactive(&underline))
}

/// Key-value pair with aligned label (padded to width).
pub fn key_value(key: &str, value: &str, label_width: usize) -> String {
    format!("{:<width$} {}", label(key), value, width = label_width)
}

/// Success check mark (respects emoji setting). Single visual cell so
/// rows mixing different mark kinds line up vertically.
pub fn check_mark() -> &'static str {
    if is_emoji_enabled() {
        "\u{2714} "
    } else {
        "\u{2713} "
    }
}

/// Failure cross mark (respects emoji setting). Single visual cell.
pub fn cross_mark() -> &'static str {
    if is_emoji_enabled() {
        "\u{2718} "
    } else {
        "\u{2717} "
    }
}

/// Warning indicator (respects emoji setting). Single visual cell.
pub fn warn_mark() -> &'static str {
    if is_emoji_enabled() {
        "\u{26a0}\u{fe0f} "
    } else {
        "! "
    }
}

/// Empty checkbox: indicates an inactive / not-installed slot. Same
/// width as `check_mark` so columns line up across rows.
pub fn empty_mark() -> &'static str {
    "  "
}

/// Wrap text in success color (green).
pub fn success(text: &str) -> String {
    wrap(SUCCESS, text)
}

/// Wrap text in warning color (yellow).
pub fn warning(text: &str) -> String {
    wrap(WARNING, text)
}

/// Wrap text in info color (cyan).
#[allow(dead_code)]
pub fn info(text: &str) -> String {
    wrap(INFO, text)
}

/// Pluralize: "1 item" vs "3 items". Handles regular plurals (append "s")
/// and custom forms ("1 cherry" / "3 cherries").
pub fn plural(count: usize, singular: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {singular}s")
    }
}
