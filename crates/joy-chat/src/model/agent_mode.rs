// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The agent permission mode (ADR JAPP-00F3-E8, revised by JI-0166-D8):
//! the MECHANICS vocabulary of AI tools and ACP. Since Interaction Levels
//! 2.0 it is never persisted in joy data; adapters derive it one-way from
//! the effective interaction level at setup and turn time. This is the
//! single canonical definition — the desktop app consumes it as the
//! ts-rs-generated `AgentMode` TS union (enable the `ts` cargo feature),
//! the platform uses it directly at the ACP boundary.

use serde::{Deserialize, Serialize};

/// Permission mode of an AI tool session (ADR JAPP-0032): how far a turn
/// may act on its own before asking a human.
///
/// The variant order is the permission lattice — most restrictive first,
/// `Plan < AcceptEdits < Autonomous`. Do not reorder variants.
///
/// Not to be confused with [`joy_core::model::config::InteractionLevel`],
/// the governed axis this mode is DERIVED from (one-way, see
/// [`from_level`]); the level's `Autonomous` variant sorts LOWEST.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum AgentMode {
    /// Read and propose only; every action needs an explicit go.
    Plan,
    /// May edit files; destructive or far-reaching actions still ask.
    AcceptEdits,
    /// Acts without asking, within the participant's reach.
    Autonomous,
}

impl std::fmt::Display for AgentMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plan => write!(f, "plan"),
            Self::AcceptEdits => write!(f, "accept-edits"),
            Self::Autonomous => write!(f, "autonomous"),
        }
    }
}

impl std::str::FromStr for AgentMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "plan" => Ok(Self::Plan),
            "accept-edits" => Ok(Self::AcceptEdits),
            "autonomous" => Ok(Self::Autonomous),
            _ => Err(format!("unknown agent mode: {s}")),
        }
    }
}

/// Derive the neutral agent mode from an effective interaction level
/// (JI-0166-D8 §4). One-way by design: never parse a mode back into a
/// level, never persist the result. Resolve the level first with
/// [`crate::model::interaction::effective_level`].
pub fn from_level(level: joy_core::model::config::InteractionLevel) -> AgentMode {
    use joy_core::model::config::InteractionLevel;
    match level {
        InteractionLevel::Proposing => AgentMode::Plan,
        InteractionLevel::Confirmed => AgentMode::AcceptEdits,
        InteractionLevel::Autonomous => AgentMode::Autonomous,
    }
}

#[cfg(test)]
mod tests {
    use super::AgentMode::{AcceptEdits, Autonomous, Plan};
    use super::*;

    #[test]
    fn ordering_is_the_permission_lattice() {
        assert!(Plan < AcceptEdits);
        assert!(AcceptEdits < Autonomous);
    }

    #[test]
    fn serde_uses_kebab_case_wire_values() {
        for (mode, wire) in [
            (Plan, "plan"),
            (AcceptEdits, "accept-edits"),
            (Autonomous, "autonomous"),
        ] {
            assert_eq!(serde_yaml_ng::to_string(&mode).unwrap().trim(), wire);
            let back: AgentMode = serde_yaml_ng::from_str(wire).unwrap();
            assert_eq!(back, mode);
        }
    }

    #[test]
    fn display_and_from_str_round_trip() {
        for mode in [Plan, AcceptEdits, Autonomous] {
            assert_eq!(mode.to_string().parse::<AgentMode>().unwrap(), mode);
        }
        assert_eq!("Accept-Edits".parse::<AgentMode>().unwrap(), AcceptEdits);
        assert_eq!(
            "yolo".parse::<AgentMode>().unwrap_err(),
            "unknown agent mode: yolo"
        );
    }

    #[test]
    fn from_level_is_the_one_way_derivation() {
        use joy_core::model::config::InteractionLevel;
        assert_eq!(from_level(InteractionLevel::Proposing), Plan);
        assert_eq!(from_level(InteractionLevel::Confirmed), AcceptEdits);
        assert_eq!(from_level(InteractionLevel::Autonomous), Autonomous);
    }
}
