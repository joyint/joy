// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use clap::Args;
use std::io::{IsTerminal, Write};
use std::process::{Command, Stdio};

use termimad::MadSkin;

// The canonical Tutorial lives at docs/user/Tutorial.md at the
// repo root. We ship an in-crate copy at crates/joy-cli/docs/
// Tutorial.md because `cargo package` builds the crate in isolation
// and cannot reach files outside the crate root. The two files must
// stay byte-identical; `just sync-tutorial` refreshes the copy and a
// unit test below catches drift. See JOY-017F-FD.
const TUTORIAL: &str = include_str!("../../docs/user/Tutorial.md");

#[derive(Args)]
pub struct TutorialArgs {
    /// Browse the tutorial via a chapter / subchapter menu (TTY only).
    #[arg(short = 'i', long)]
    interactive: bool,
}

pub fn run(args: TutorialArgs) -> Result<()> {
    run_markdown(TUTORIAL, args.interactive, true)
}

/// Render any tutorial-style markdown through the same interactive
/// chapter menu path used by `joy tutorial`. Exposed so `joy ai tutorial`
/// and any future tutorial-shaped command can reuse the renderer.
///
/// `use_pager` controls the non-interactive rendering: `true` follows the
/// human-tutorial default (pager when stdout is a TTY, plain otherwise);
/// `false` always prints plain text. `joy ai tutorial` passes `false` so
/// AI tool runners that wire stdio through a PTY still get clean output
/// without a `less` instance taking over.
pub(crate) fn run_markdown(markdown: &str, interactive: bool, use_pager: bool) -> Result<()> {
    if interactive && std::io::stdout().is_terminal() {
        return run_interactive(markdown);
    }
    print_full(markdown, use_pager)
}

fn print_full(markdown: &str, use_pager: bool) -> Result<()> {
    let width = crate::color::terminal_width();
    let is_tty = std::io::stdout().is_terminal();

    // ANSI codes only make sense if stdout is going to a real
    // terminal. When piped or captured (`joy tutorial > file`,
    // `joy ai tutorial | less`, AI tool capture, CI), render with
    // an unstyled skin so the output is plain readable markdown
    // and not raw escape sequences.
    let skin = if is_tty {
        MadSkin::default()
    } else {
        MadSkin::no_style()
    };
    let formatted = skin.area_text(markdown, &termimad::Area::new(0, 0, width as u16, u16::MAX));
    let output = formatted.to_string();

    // Pager only when explicitly requested AND a human is reading.
    // When stdout is piped or captured, or when the caller opted out
    // of the pager (`joy ai tutorial`), dump the rendered text
    // directly so callers get a clean byte stream without `less`
    // trying to seize a non-existent TTY.
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

fn run_interactive(markdown: &str) -> Result<()> {
    let chapters = parse_chapters(markdown);
    if chapters.is_empty() {
        // Defensive: if parsing ever fails, fall back to the dump path.
        return print_full(markdown, true);
    }
    let title = document_title(markdown).unwrap_or("Joy Tutorial");

    loop {
        let labels: Vec<String> = chapters.iter().map(|c| c.title.clone()).collect();
        let choice = inquire::Select::new(title, labels)
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
    // Route the rendered section through a pager so the user can
    // scroll with arrows / PgUp / PgDn / `/`-search; quitting the
    // pager (`q`) returns to the menu loop above.
    print_full(markdown, true)
}

#[cfg(test)]
mod tests {
    /// The Tutorial lives in two places at once: docs/user/Tutorial.md
    /// is the canonical doc, crates/joy-cli/docs/user/Tutorial.md is
    /// shipped inside the crate so cargo package can find it. They
    /// must stay byte-identical. If this test fails, run
    /// `just sync-tutorial` from the repo root.
    #[test]
    fn in_crate_tutorial_matches_canonical() {
        let canonical = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/user/Tutorial.md"
        ));
        let shipped = super::TUTORIAL;
        assert_eq!(
            canonical, shipped,
            "crates/joy-cli/docs/user/Tutorial.md is out of sync with \
             docs/user/Tutorial.md. Run `just sync-tutorial`."
        );
    }
}
