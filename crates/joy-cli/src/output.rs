// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

// Items become "used" once subcommands start consuming them
// (JOY-0123-9A and following). Until then the helpers compile-clean
// without triggering -D warnings on the bin crate.
#![allow(dead_code)]

//! Cross-cutting output mode for joy-cli (ADR-036 §1).
//!
//! Every command runs in one of two output modes:
//!
//! - `OutputMode::Display`: human-readable terminal output. Format may
//!   evolve freely between versions. CIs must not parse this.
//! - `OutputMode::Json`: machine-readable JSON envelope. Stable
//!   contract: `{"version": 1, "data": ...}`. Additive-only between
//!   minor versions; breaking changes go through a deprecation cycle.
//!
//! The mode is selected by the global `--json` flag on `Cli` and
//! installed once at startup via [`set_mode`]. Commands query it via
//! [`mode`] and emit JSON via [`emit`].

use std::sync::atomic::{AtomicU8, Ordering};

use serde::Serialize;

/// Schema version emitted in every JSON envelope. Bump only on
/// breaking changes; additive changes keep the same version.
pub const SCHEMA_VERSION: u32 = 1;

/// Output surface a command should produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Display,
    Json,
}

// Process-wide mode, set once at startup. Stored as u8 for AtomicU8.
const MODE_DISPLAY: u8 = 0;
const MODE_JSON: u8 = 1;
static MODE: AtomicU8 = AtomicU8::new(MODE_DISPLAY);

/// Install the output mode for this process. Called from main.rs once.
pub fn set_mode(mode: OutputMode) {
    let v = match mode {
        OutputMode::Display => MODE_DISPLAY,
        OutputMode::Json => MODE_JSON,
    };
    MODE.store(v, Ordering::Relaxed);
}

/// Read the current output mode.
pub fn mode() -> OutputMode {
    match MODE.load(Ordering::Relaxed) {
        MODE_JSON => OutputMode::Json,
        _ => OutputMode::Display,
    }
}

/// True iff JSON mode is active.
pub fn is_json() -> bool {
    mode() == OutputMode::Json
}

/// JSON envelope wrapping every machine-readable payload.
#[derive(Serialize)]
pub struct Envelope<T: Serialize> {
    pub version: u32,
    pub data: T,
}

/// Emit `data` as a JSON envelope to stdout, with a trailing newline.
/// Only call this from JSON mode; Display-mode commands keep their
/// existing print paths.
pub fn emit<T: Serialize>(data: T) -> anyhow::Result<()> {
    let envelope = Envelope {
        version: SCHEMA_VERSION,
        data,
    };
    // Serialize in presentation mode so any MemberRef field resolves to the
    // name/e-mail (or an auth request), identical to the terminal, and never
    // emits a raw opaque id (ADR-042). On-disk writes do not go through here, so
    // they keep persisting the raw id.
    let s = joy_core::member_ref::with_presentation(|| serde_json::to_string(&envelope))?;
    println!("{s}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct Sample {
        id: String,
        count: u32,
    }

    #[test]
    fn envelope_serialises_with_version_and_data() {
        let env = Envelope {
            version: SCHEMA_VERSION,
            data: Sample {
                id: "JOY-0001".to_string(),
                count: 3,
            },
        };
        let s = serde_json::to_string(&env).unwrap();
        assert!(s.contains(r#""version":1"#));
        assert!(s.contains(r#""id":"JOY-0001""#));
        assert!(s.contains(r#""count":3"#));
    }

    #[test]
    fn mode_round_trips() {
        set_mode(OutputMode::Json);
        assert_eq!(mode(), OutputMode::Json);
        assert!(is_json());
        set_mode(OutputMode::Display);
        assert_eq!(mode(), OutputMode::Display);
        assert!(!is_json());
    }
}
