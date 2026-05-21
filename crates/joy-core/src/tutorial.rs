// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Shared tutorial / markdown rendering and interactive chapter browsing.
//!
//! This module is reused by every Joyint CLI that ships a tutorial
//! (`joy tutorial`, `joy ai tutorial`, `jyn tutorial`, ...) and by any
//! command that renders markdown for the terminal (e.g. item
//! descriptions in `joy show`). It is gated behind the `tutorial`
//! cargo feature so library-only consumers (joyint-server) do not pull
//! in terminal UI crates (termimad, inquire, terminal_size).
//!
//! Functions return [`std::io::Result`] rather than a richer error type
//! so the module stays free of `anyhow`; CLI callers convert with `?`.

use std::io::{self, IsTerminal, Write};
use std::process::{Command, Stdio};

use termimad::MadSkin;

/// Detect terminal width. Checks the COLUMNS environment variable first
/// (standard override, respected by git and friends, and makes tests
/// deterministic) before falling back to the OS-reported terminal size
/// and finally to 80 columns.
pub fn terminal_width() -> usize {
    if let Some(w) = std::env::var("COLUMNS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&w| w > 0)
    {
        return w;
    }
    if let Some((terminal_size::Width(w), _)) = terminal_size::terminal_size() {
        return w as usize;
    }
    80
}

/// Render tutorial-style markdown, either through the interactive
/// chapter menu (TTY + `interactive`) or as a single rendered document.
///
/// `use_pager` controls the non-interactive rendering: `true` follows the
/// human-tutorial default (pager when stdout is a TTY, plain otherwise);
/// `false` always prints plain text. AI-facing tutorials pass `false` so
/// tool runners that wire stdio through a PTY still get clean output
/// without a `less` instance taking over.
pub fn run_markdown(markdown: &str, interactive: bool, use_pager: bool) -> io::Result<()> {
    if interactive && std::io::stdout().is_terminal() {
        return run_interactive(markdown);
    }
    print_full(markdown, use_pager)
}

/// Render any markdown string for display in the terminal. Picks a
/// styled skin when stdout is a TTY and an unstyled one otherwise, so
/// piped output stays free of ANSI escapes. Shared with markdown-capable
/// output paths such as `joy show` (item descriptions, comment bodies).
pub fn render_markdown(markdown: &str) -> String {
    render_markdown_with_width(markdown, terminal_width())
}

/// Render markdown wrapped to `width` columns rather than the full
/// terminal. Useful when the caller will prefix each rendered line with
/// an indent: caller passes `terminal_width - indent_width` so the
/// indent + line stays within the terminal and termimad's wrapping does
/// not produce overflowing lines that bleed back to column zero.
pub fn render_markdown_with_width(markdown: &str, width: usize) -> String {
    let is_tty = std::io::stdout().is_terminal();
    let skin = if is_tty {
        MadSkin::default()
    } else {
        MadSkin::no_style()
    };
    let formatted = skin.area_text(markdown, &termimad::Area::new(0, 0, width as u16, u16::MAX));
    formatted.to_string()
}

fn print_full(markdown: &str, use_pager: bool) -> io::Result<()> {
    let is_tty = std::io::stdout().is_terminal();
    let output = render_markdown(markdown);

    // Pager only when explicitly requested AND a human is reading.
    // When stdout is piped or captured, or when the caller opted out
    // of the pager, dump the rendered text directly so callers get a
    // clean byte stream without `less` trying to seize a non-existent
    // TTY.
    if !use_pager || !is_tty {
        print!("{output}");
        return Ok(());
    }

    let pager = std::env::var("PAGER").ok().unwrap_or_default();
    let pagers = if pager.is_empty() {
        vec!["less -R", "more"]
    } else {
        vec![pager.as_str(), "less -R", "more"]
    };

    for p in &pagers {
        let parts: Vec<&str> = p.split_whitespace().collect();
        let (cmd, args) = match parts.split_first() {
            Some((c, a)) => (*c, a),
            None => continue,
        };
        let mut child = match Command::new(cmd).args(args).stdin(Stdio::piped()).spawn() {
            Ok(c) => c,
            Err(_) => continue,
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(output.as_bytes());
        }
        let _ = child.wait();
        return Ok(());
    }

    print!("{output}");
    Ok(())
}

/// Parsed tutorial structure: ordered list of top-level chapters, each
/// chapter carries its body slice plus an ordered list of subsections.
struct Chapter {
    title: String,
    body: String,
    subsections: Vec<Subsection>,
}

struct Subsection {
    title: String,
    body: String,
}

/// Return the first `# Title` line of the document, or `None` if there is
/// no top-level heading.
fn document_title(markdown: &str) -> Option<&str> {
    markdown.lines().find_map(|line| line.strip_prefix("# "))
}

/// Walk the Markdown line by line, collecting `## Chapter` blocks and
/// the `### Subsection` blocks nested inside. The body of each chapter
/// is every line up to the next heading of the same or higher level.
/// Subsection bodies stop at the next `##` or `###`. The leading `#`
/// document title is skipped (chapters cover all useful content).
fn parse_chapters(markdown: &str) -> Vec<Chapter> {
    let mut chapters: Vec<Chapter> = Vec::new();
    let mut current_chapter: Option<Chapter> = None;
    let mut current_subsection: Option<Subsection> = None;

    let flush_subsection = |chap: &mut Chapter, sub: &mut Option<Subsection>| {
        if let Some(s) = sub.take() {
            chap.subsections.push(s);
        }
    };
    let push_line_to = |dest: &mut String, line: &str| {
        dest.push_str(line);
        dest.push('\n');
    };

    for line in markdown.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            if let Some(mut chap) = current_chapter.take() {
                flush_subsection(&mut chap, &mut current_subsection);
                chapters.push(chap);
            }
            current_chapter = Some(Chapter {
                title: rest.trim().to_string(),
                body: format!("## {rest}\n"),
                subsections: Vec::new(),
            });
        } else if let Some(rest) = line.strip_prefix("### ") {
            if let Some(chap) = current_chapter.as_mut() {
                flush_subsection(chap, &mut current_subsection);
                current_subsection = Some(Subsection {
                    title: rest.trim().to_string(),
                    body: format!("### {rest}\n"),
                });
            }
        } else if line.starts_with("# ") {
            // Document title - skip.
            continue;
        } else if let Some(sub) = current_subsection.as_mut() {
            push_line_to(&mut sub.body, line);
            if let Some(chap) = current_chapter.as_mut() {
                push_line_to(&mut chap.body, line);
            }
        } else if let Some(chap) = current_chapter.as_mut() {
            push_line_to(&mut chap.body, line);
        }
    }
    if let Some(mut chap) = current_chapter.take() {
        flush_subsection(&mut chap, &mut current_subsection);
        chapters.push(chap);
    }
    chapters
}

fn run_interactive(markdown: &str) -> io::Result<()> {
    let chapters = parse_chapters(markdown);
    if chapters.is_empty() {
        // Defensive: if parsing ever fails, fall back to the dump path.
        return print_full(markdown, true);
    }
    let title = document_title(markdown).unwrap_or("Tutorial");

    loop {
        let labels: Vec<String> = chapters.iter().map(|c| c.title.clone()).collect();
        let choice = inquire::Select::new(title, labels)
            .with_help_message("Enter: open chapter   Esc: quit")
            .prompt_skippable()
            .map_err(to_io)?;
        let Some(label) = choice else {
            return Ok(());
        };
        let Some(chapter) = chapters.iter().find(|c| c.title == label) else {
            continue;
        };
        chapter_loop(chapter)?;
    }
}

fn chapter_loop(chapter: &Chapter) -> io::Result<()> {
    if chapter.subsections.is_empty() {
        return render_then_pause(&chapter.body);
    }
    loop {
        let mut labels: Vec<String> = Vec::with_capacity(chapter.subsections.len() + 1);
        labels.push("<full chapter>".to_string());
        labels.extend(chapter.subsections.iter().map(|s| s.title.clone()));
        let prompt = format!("{} - pick a section", chapter.title);
        let choice = inquire::Select::new(&prompt, labels)
            .with_help_message("Enter: open section   Esc: back")
            .prompt_skippable()
            .map_err(to_io)?;
        let Some(label) = choice else {
            return Ok(());
        };
        if label == "<full chapter>" {
            render_then_pause(&chapter.body)?;
            continue;
        }
        if let Some(sub) = chapter.subsections.iter().find(|s| s.title == label) {
            render_then_pause(&sub.body)?;
        }
    }
}

fn render_then_pause(markdown: &str) -> io::Result<()> {
    // Route the rendered section through a pager so the user can scroll
    // with arrows / PgUp / PgDn / `/`-search; quitting the pager (`q`)
    // returns to the menu loop above.
    print_full(markdown, true)
}

/// Map an inquire prompt error into an `io::Error` so this module can
/// stay free of the `anyhow` dependency.
fn to_io(e: inquire::InquireError) -> io::Error {
    io::Error::other(e)
}
