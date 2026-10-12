// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The strength line above a passphrase prompt (JI-01A5-93,
//! JOY-02CC-46): five squares and the level with the entropy in
//! brackets, redrawn on every key while a NEW passphrase is typed.
//! Nothing is echoed; the line is what the person sees of their typing.
//!
//! [`read`] is the whole of the behaviour over any key source and any
//! writer, so a test drives it with recorded keys and reads the drawn
//! lines. [`prompt`] is the terminal around it: raw mode, crossterm's
//! key events, stderr.

use std::io::{self, Write};

use joy_passphrase::{strength, Level, Strength, RISK_SENTENCE};
use zeroize::Zeroizing;

/// What a key does here. Anything else a terminal sends is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Backspace,
    /// Ctrl-U: start over.
    ClearLine,
    Enter,
    /// Ctrl-C or Ctrl-D: no passphrase.
    Cancel,
}

const RESET: &str = "\x1b[0m";

/// What stands above the prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Draw {
    /// The strength line, redrawn on every key.
    Meter { color: bool },
    /// Nothing: the confirmation of a passphrase just chosen. It reads
    /// from the same key source as the meter, so what a person typed
    /// ahead of the second prompt reaches it (a hidden line read from
    /// the terminal on its own would never see keys the first prompt
    /// had already taken in).
    PromptOnly,
}

/// The bar's colour per level: red, orange, yellow, light green, dark
/// green, as 256-colour codes. A terminal without them shows its
/// nearest; the word next to the bar carries the level in any case.
fn colour(level: Level) -> &'static str {
    match level {
        Level::Insufficient => "\x1b[38;5;196m",
        Level::Sufficient => "\x1b[38;5;208m",
        Level::Good => "\x1b[38;5;220m",
        Level::VeryGood => "\x1b[38;5;119m",
        Level::Excellent => "\x1b[38;5;28m",
    }
}

/// The line: `■■■□□ good (entropy 47 bit)`; the words are
/// joy-passphrase's, the same the app shows.
pub fn line(s: &Strength, color: bool) -> String {
    // nothing typed, nothing filled; a level needs at least one character
    let filled = if s.bits == 0.0 {
        0
    } else {
        usize::from(s.level.rank())
    };
    let bar: String = (0..5).map(|i| if i < filled { '■' } else { '□' }).collect();
    let text = s.text();
    if color {
        format!("{}{bar}{RESET} {text}", colour(s.level))
    } else {
        format!("{bar} {text}")
    }
}

/// Read a passphrase from `keys`, drawing on `out`: with [`Draw::Meter`]
/// the line, then the prompt below it, both redrawn on every key; with
/// [`Draw::PromptOnly`] the prompt alone. What was drawn is erased
/// before returning, so nothing of the entry lingers. `None` when the
/// person cancelled or the keys ended before Enter.
///
/// Enter on an insufficient passphrase (operator 2026-10-12) does not
/// take it: the prompt line asks `I know and accept the risk. (y/N)`,
/// `y` takes it, anything else starts the entry over.
pub fn read(
    prompt: &str,
    keys: impl IntoIterator<Item = Key>,
    out: &mut impl Write,
    draw: Draw,
) -> io::Result<Option<Zeroizing<String>>> {
    let mut keys = keys.into_iter();
    let mut typed = Zeroizing::new(String::new());
    match draw {
        Draw::Meter { color } => write!(out, "{}\r\n{prompt}", line(&strength(""), color))?,
        Draw::PromptOnly => write!(out, "{prompt}")?,
    }
    out.flush()?;
    while let Some(key) = keys.next() {
        match key {
            Key::Char(c) => typed.push(c),
            Key::Backspace => {
                typed.pop();
            }
            Key::ClearLine => typed.clear(),
            Key::Enter => {
                if matches!(draw, Draw::Meter { .. })
                    && strength(&typed).needs_risk_acknowledgement()
                {
                    write!(out, "\r\x1b[2K{RISK_SENTENCE} (y/N) ")?;
                    out.flush()?;
                    match keys.next() {
                        Some(Key::Char('y')) | Some(Key::Char('Y')) => {}
                        Some(Key::Cancel) | None => {
                            erase(out, draw)?;
                            return Ok(None);
                        }
                        Some(_) => {
                            typed.clear();
                            if let Draw::Meter { color } = draw {
                                write!(
                                    out,
                                    "\x1b[1A\r\x1b[2K{}\r\n\x1b[2K{prompt}",
                                    line(&strength(""), color)
                                )?;
                                out.flush()?;
                            }
                            continue;
                        }
                    }
                }
                erase(out, draw)?;
                return Ok(Some(typed));
            }
            Key::Cancel => {
                erase(out, draw)?;
                return Ok(None);
            }
        }
        if let Draw::Meter { color } = draw {
            // up to the line, redraw it, down again, redraw the prompt
            write!(
                out,
                "\x1b[1A\r\x1b[2K{}\r\n\x1b[2K{prompt}",
                line(&strength(&typed), color)
            )?;
            out.flush()?;
        }
    }
    erase(out, draw)?;
    Ok(None)
}

/// Clear the prompt line, and the line above it when the meter stood
/// there; the cursor ends at column 0 of the first cleared line, so the
/// command's next output takes its place.
fn erase(out: &mut impl Write, draw: Draw) -> io::Result<()> {
    match draw {
        Draw::Meter { .. } => write!(out, "\r\x1b[2K\x1b[1A\r\x1b[2K")?,
        Draw::PromptOnly => write!(out, "\r\x1b[2K")?,
    }
    out.flush()
}

/// Ask for a new passphrase at the terminal, the strength line above
/// the prompt. The caller has checked that stdin is a terminal.
pub fn prompt(prompt: &str) -> anyhow::Result<String> {
    at_the_terminal(
        prompt,
        Draw::Meter {
            color: crate::color::enabled(),
        },
    )
}

/// Ask for the confirmation of a passphrase just chosen: the prompt
/// alone, over the same keys as [`prompt`].
pub fn confirm(prompt: &str) -> anyhow::Result<String> {
    at_the_terminal(prompt, Draw::PromptOnly)
}

/// Raw mode for the keys, stderr for the drawing.
fn at_the_terminal(prompt: &str, draw: Draw) -> anyhow::Result<String> {
    use anyhow::Context;
    let raw = RawMode::enable().context("the terminal could not be put into raw mode")?;
    let mut keys = TerminalKeys { failed: None };
    let mut err = io::stderr().lock();
    let typed = read(prompt, &mut keys, &mut err, draw)
        .context("the passphrase prompt could not be drawn")?;
    drop(raw);
    match (typed, keys.failed) {
        (Some(typed), _) => Ok(typed.to_string()),
        (None, Some(e)) => Err(e).context("the terminal's keys could not be read"),
        (None, None) => anyhow::bail!("no passphrase was entered"),
    }
}

/// Raw mode for as long as this lives; an error on the way out leaves
/// the terminal in raw mode otherwise.
struct RawMode;

impl RawMode {
    fn enable() -> io::Result<RawMode> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(RawMode)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// crossterm's key events as [`Key`]s; ends when the terminal does,
/// keeping the error that ended it.
struct TerminalKeys {
    failed: Option<io::Error>,
}

impl Iterator for TerminalKeys {
    type Item = Key;

    fn next(&mut self) -> Option<Key> {
        use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
        loop {
            let event = match crossterm::event::read() {
                Ok(event) => event,
                Err(e) => {
                    self.failed = Some(e);
                    return None;
                }
            };
            let Event::Key(key) = event else {
                continue;
            };
            if key.kind == KeyEventKind::Release {
                continue;
            }
            let control = key.modifiers.contains(KeyModifiers::CONTROL);
            return Some(match (key.code, control) {
                (KeyCode::Char('c'), true) | (KeyCode::Char('d'), true) => Key::Cancel,
                (KeyCode::Char('u'), true) => Key::ClearLine,
                // a line feed in raw mode arrives as Ctrl-J
                (KeyCode::Char('j'), true) => Key::Enter,
                (KeyCode::Char(c), _) => Key::Char(c),
                (KeyCode::Backspace, _) => Key::Backspace,
                (KeyCode::Enter, _) => Key::Enter,
                _ => continue,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> Vec<Key> {
        text.chars().map(Key::Char).collect()
    }

    fn drawn(keys: Vec<Key>, color: bool) -> (Option<String>, String) {
        let mut out = Vec::new();
        let got = read("Passphrase: ", keys, &mut out, Draw::Meter { color }).unwrap();
        (got.map(|z| z.to_string()), String::from_utf8(out).unwrap())
    }

    #[test]
    fn the_line_shows_the_bar_the_level_and_the_bits() {
        let s = Strength {
            bits: 46.6,
            level: Level::Good,
        };
        assert_eq!(line(&s, false), "■■■□□ good (entropy 47 bit)");
        assert_eq!(
            line(&s, true),
            "\x1b[38;5;220m■■■□□\x1b[0m good (entropy 47 bit)"
        );
        let empty = strength("");
        assert_eq!(line(&empty, false), "□□□□□ insufficient (entropy 0 bit)");
    }

    #[test]
    fn every_key_redraws_the_line_and_enter_returns_what_was_typed() {
        let mut keys = typed("q w e");
        keys.push(Key::Enter);
        keys.push(Key::Char('y'));
        let (got, out) = drawn(keys, false);
        assert_eq!(got.as_deref(), Some("q w e"));
        assert!(
            out.contains("I know and accept the risk. (y/N) "),
            "{out:?}"
        );
        // the first drawing, before any key
        assert!(out.starts_with("□□□□□ insufficient (entropy 0 bit)\r\nPassphrase: "));
        // one redraw per key: up, clear, line, down, clear, prompt
        let redraws: Vec<&str> = out.split("\x1b[1A\r\x1b[2K").skip(1).collect();
        assert_eq!(redraws.len(), 6, "five redraws and the erase: {out:?}");
        assert!(
            redraws[4].starts_with("■□□□□ insufficient (entropy 17 bit)\r\n\x1b[2KPassphrase: "),
            "{:?}",
            redraws[4]
        );
        // nothing typed is ever echoed
        assert!(!out.contains("q w e"));
        // and both lines are gone at the end
        assert!(out.ends_with("\r\x1b[2K\x1b[1A\r\x1b[2K"));
    }

    #[test]
    fn the_bar_fills_as_the_passphrase_grows() {
        let mut keys = typed("ladle quartz nimbus fjord carapace velvet");
        keys.push(Key::Enter);
        let (got, out) = drawn(keys, false);
        assert_eq!(
            got.as_deref(),
            Some("ladle quartz nimbus fjord carapace velvet")
        );
        assert!(out.contains("■■■■■ excellent (entropy >64 bit)"), "{out}");
        assert!(out.contains("■■□□□ sufficient (entropy"), "{out}");
        // a strong passphrase is taken without a question
        assert!(!out.contains("accept the risk"), "{out:?}");
    }

    #[test]
    fn an_insufficient_passphrase_is_taken_with_a_y_and_started_over_without() {
        // N (the default, any other key) starts over: the line is back at
        // nothing typed, and what follows is a new entry
        let mut keys = typed("q w e");
        keys.push(Key::Enter);
        keys.push(Key::Enter);
        keys.extend(typed("ladle quartz nimbus fjord"));
        keys.push(Key::Enter);
        let (got, out) = drawn(keys, false);
        assert_eq!(got.as_deref(), Some("ladle quartz nimbus fjord"));
        assert!(
            out.contains("I know and accept the risk. (y/N) "),
            "{out:?}"
        );
        let after = out.rsplit("(y/N) ").next().unwrap();
        assert!(
            after.contains("□□□□□ insufficient (entropy 0 bit)"),
            "{after:?}"
        );

        // cancel at the question is no passphrase
        let mut keys = typed("q w e");
        keys.push(Key::Enter);
        keys.push(Key::Cancel);
        assert_eq!(drawn(keys, false).0, None);

        // the question is the meter's; a confirmation takes what it gets
        let mut out = Vec::new();
        let mut keys = typed("q w e");
        keys.push(Key::Enter);
        let got = read("Confirm: ", keys, &mut out, Draw::PromptOnly).unwrap();
        assert_eq!(got.unwrap().to_string(), "q w e");
        assert!(!String::from_utf8(out).unwrap().contains("risk"));
    }

    #[test]
    fn backspace_and_clear_line_edit_what_is_typed() {
        let mut keys = typed("abc");
        keys.push(Key::Backspace);
        keys.extend(typed("d"));
        keys.push(Key::Enter);
        keys.push(Key::Char('y'));
        assert_eq!(drawn(keys, false).0.as_deref(), Some("abd"));

        let mut keys = typed("abc");
        keys.push(Key::ClearLine);
        keys.extend(typed("z"));
        keys.push(Key::Enter);
        keys.push(Key::Char('y'));
        assert_eq!(drawn(keys, false).0.as_deref(), Some("z"));

        // backspace on nothing is nothing
        let keys = vec![Key::Backspace, Key::Enter];
        assert_eq!(drawn(keys, false).0.as_deref(), Some(""));
    }

    #[test]
    fn cancel_and_a_closed_key_source_give_no_passphrase() {
        let mut keys = typed("secret");
        keys.push(Key::Cancel);
        let (got, out) = drawn(keys, false);
        assert_eq!(got, None);
        assert!(out.ends_with("\r\x1b[2K\x1b[1A\r\x1b[2K"));

        let (got, _) = drawn(typed("secret"), false);
        assert_eq!(got, None);
    }

    #[test]
    fn the_confirmation_draws_the_prompt_alone_and_erases_it() {
        let mut keys = typed("q w e");
        keys.push(Key::Enter);
        let mut out = Vec::new();
        let got = read("Confirm: ", keys, &mut out, Draw::PromptOnly).unwrap();
        assert_eq!(got.unwrap().to_string(), "q w e");
        assert_eq!(String::from_utf8(out).unwrap(), "Confirm: \r\x1b[2K");
    }

    #[test]
    fn colour_sits_on_the_bar_alone() {
        let mut keys = typed("q w e");
        keys.push(Key::Enter);
        keys.push(Key::Char('y'));
        let (_, out) = drawn(keys, true);
        assert!(
            out.contains("\x1b[38;5;196m■□□□□\x1b[0m insufficient"),
            "{out:?}"
        );
    }
}
