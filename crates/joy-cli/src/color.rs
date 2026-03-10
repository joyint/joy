// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::io::IsTerminal;
use std::sync::OnceLock;

use joy_core::model::item::{ItemType, Priority, Status};

static ENABLED: OnceLock<bool> = OnceLock::new();

fn is_enabled() -> bool {
    *ENABLED.get_or_init(|| {
        if std::env::var_os("NO_COLOR").is_some() {
            return false;
        }
        std::io::stdout().is_terminal()
    })
}

// ANSI sequences using only the basic 8 colors (0-7).
// These map to the terminal's color theme, so they adapt to light/dark modes.
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const MAGENTA: &str = "\x1b[35m";
const CYAN: &str = "\x1b[36m";

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
    wrap(BOLD, text)
}

pub fn status(s: &Status) -> String {
    let text = s.to_string();
    match s {
        Status::New => text,
        Status::Open => wrap(BLUE, &text),
        Status::InProgress => wrap(YELLOW, &text),
        Status::Review => wrap(CYAN, &text),
        Status::Closed => wrap(GREEN, &text),
        Status::Deferred => wrap(DIM, &text),
    }
}

pub fn priority(p: &Priority) -> String {
    let text = p.to_string();
    match p {
        Priority::Critical => wrap2(BOLD, RED, &text),
        Priority::High => wrap(RED, &text),
        Priority::Medium => wrap(YELLOW, &text),
        Priority::Low => text,
    }
}

pub fn item_type(t: &ItemType) -> String {
    let text = t.to_string();
    match t {
        ItemType::Epic => wrap(MAGENTA, &text),
        ItemType::Bug => wrap(RED, &text),
        _ => wrap(DIM, &text),
    }
}

pub fn blocked(text: &str) -> String {
    wrap(RED, text)
}

pub fn label(text: &str) -> String {
    wrap(DIM, text)
}

pub fn heading(text: &str) -> String {
    wrap(BOLD, text)
}

pub fn text(text: &str) -> String {
    wrap(DIM, text)
}

pub fn status_heading(s: &Status, text: &str) -> String {
    match s {
        Status::New => wrap(BOLD, text),
        Status::Open => wrap2(BOLD, BLUE, text),
        Status::InProgress => wrap2(BOLD, YELLOW, text),
        Status::Review => wrap2(BOLD, CYAN, text),
        Status::Closed => wrap2(BOLD, GREEN, text),
        Status::Deferred => wrap(DIM, text),
    }
}
