// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::io::IsTerminal;
use std::sync::OnceLock;

use joy_core::model::config::{ColorMode, OutputConfig};
use joy_core::model::item::{ItemType, Priority, Status};

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
        ItemType::Rework => "\u{267b}\u{fe0f} ",
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
        Status::InProgress => "\u{25b6}\u{fe0f} ",
        Status::Review => "\u{1f440} ",
        Status::Closed => "\u{2705} ",
        Status::Deferred => "\u{23f8}\u{fe0f} ",
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

fn item_type_colored_short(t: &ItemType) -> String {
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

fn priority_colored_short(p: &Priority) -> String {
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
        Priority::High => "hi",
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

/// Format a size value (1-7) as a colored block character.
/// Always shown, regardless of emoji setting.
/// Colors: 1-2 green, 3-4 yellow, 5 orange, 6-7 red.
pub fn size_indicator(size: Option<u8>) -> String {
    const GREEN: &str = "\x1b[32m";
    const YELLOW: &str = "\x1b[33m";
    const ORANGE: &str = "\x1b[38;5;208m";
    const RED: &str = "\x1b[31m";
    const RESET: &str = "\x1b[0m";

    let blocks = ['▁', '▂', '▃', '▄', '▅', '▆', '▇'];

    match size {
        Some(s) if (1..=7).contains(&s) => {
            let block = blocks[(s - 1) as usize];
            let color = match s {
                1 | 2 => GREEN,
                3 | 4 => YELLOW,
                5 => ORANGE,
                6 | 7 => RED,
                _ => "",
            };
            if is_color_enabled() {
                format!("{color}{block}{RESET}")
            } else {
                format!("{}", s)
            }
        }
        _ => " ".to_string(),
    }
}

fn is_color_enabled() -> bool {
    *ENABLED.get().unwrap_or(&false)
}
