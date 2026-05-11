// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use clap::Args;
use std::io::{IsTerminal, Write};
use std::process::{Command, Stdio};

use termimad::MadSkin;

const TUTORIAL: &str = include_str!("../../../../docs/user/Tutorial.md");

#[derive(Args)]
pub struct TutorialArgs {
    /// Browse the tutorial via a chapter / subchapter menu (TTY only).
    #[arg(short = 'i', long)]
    interactive: bool,
}

pub fn run(args: TutorialArgs) -> Result<()> {
    if args.interactive && std::io::stdout().is_terminal() {
        return run_interactive();
    }
    print_full(TUTORIAL)
}

fn print_full(markdown: &str) -> Result<()> {
    let width = crate::color::terminal_width();
    let skin = MadSkin::default_dark();
    let formatted = skin.area_text(markdown, &termimad::Area::new(0, 0, width as u16, u16::MAX));

    let pager = std::env::var("PAGER").ok().unwrap_or_default();
    let pagers = if pager.is_empty() {
        vec!["less -R", "more"]
    } else {
        vec![pager.as_str(), "less -R", "more"]
    };

    let output = formatted.to_string();

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

/// Parsed Tutorial.md structure: ordered list of top-level chapters,
/// each chapter carries its body slice plus an ordered list of
/// subsections.
struct Chapter {
    title: String,
    body: String,
    subsections: Vec<Subsection>,
}

struct Subsection {
    title: String,
    body: String,
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

fn run_interactive() -> Result<()> {
    let chapters = parse_chapters(TUTORIAL);
    if chapters.is_empty() {
        // Defensive: if parsing ever fails, fall back to the dump path.
        return print_full(TUTORIAL);
    }

    loop {
        let labels: Vec<String> = chapters.iter().map(|c| c.title.clone()).collect();
        let choice = inquire::Select::new("Joy Tutorial", labels)
            .with_help_message("Enter: open chapter   Esc: quit")
            .prompt_skippable()?;
        let Some(label) = choice else {
            return Ok(());
        };
        let Some(chapter) = chapters.iter().find(|c| c.title == label) else {
            continue;
        };
        chapter_loop(chapter)?;
    }
}

fn chapter_loop(chapter: &Chapter) -> Result<()> {
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
            .prompt_skippable()?;
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

fn render_then_pause(markdown: &str) -> Result<()> {
    let width = crate::color::terminal_width();
    let skin = MadSkin::default_dark();
    let formatted = skin.area_text(markdown, &termimad::Area::new(0, 0, width as u16, u16::MAX));
    println!("{formatted}");
    let _ = inquire::Confirm::new("Press Enter to return to the menu")
        .with_default(true)
        .prompt_skippable();
    Ok(())
}
