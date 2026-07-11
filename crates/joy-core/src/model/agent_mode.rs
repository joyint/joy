// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The agent permission mode and its resolution lattice (ADR
//! JAPP-00F3-E8). This is the single canonical definition — the desktop
//! app consumes it as the ts-rs-generated `AgentMode` TS union (enable
//! the `ts` cargo feature), the platform uses it directly.

use serde::{Deserialize, Serialize};

/// Permission mode of an AI participant (ADR JAPP-0032): how far a turn
/// may act on its own before asking a human.
///
/// The variant order IS the permission lattice — most restrictive first,
/// `Plan < AcceptEdits < Autonomous` — so capping a mode by a ceiling is
/// plain [`Ord::min`] (see [`effective_mode`]). Do not reorder variants.
///
/// Not to be confused with [`crate::model::config::InteractionLevel`],
/// a different axis whose `Autonomous` variant sorts LOWEST.
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

/// Resolve the mode a turn actually runs under (ADR JAPP-00F3-E8): the
/// per-chat, per-delegator override when one is stored (else the
/// participant's project default), capped by the project-wide ceiling
/// when a manage member set one.
///
/// Pure lattice math on [`AgentMode`]'s `Ord`:
/// `min(ceiling or Autonomous, override or default)`.
pub fn effective_mode(
    ceiling: Option<AgentMode>,
    override_mode: Option<AgentMode>,
    default_mode: AgentMode,
) -> AgentMode {
    let chosen = override_mode.unwrap_or(default_mode);
    ceiling.map_or(chosen, |c| chosen.min(c))
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
    fn effective_mode_full_truth_table() {
        // (ceiling, override, default) -> effective, hand-computed as
        // min(ceiling or Autonomous, override or default).
        #[rustfmt::skip]
        let table: &[(Option<AgentMode>, Option<AgentMode>, AgentMode, AgentMode)] = &[
            // default = Plan
            (None,             None,             Plan, Plan),
            (Some(Plan),       None,             Plan, Plan),
            (Some(AcceptEdits),None,             Plan, Plan),
            (Some(Autonomous), None,             Plan, Plan),
            (None,             Some(Plan),       Plan, Plan),
            (Some(Plan),       Some(Plan),       Plan, Plan),
            (Some(AcceptEdits),Some(Plan),       Plan, Plan),
            (Some(Autonomous), Some(Plan),       Plan, Plan),
            (None,             Some(AcceptEdits),Plan, AcceptEdits),
            (Some(Plan),       Some(AcceptEdits),Plan, Plan),
            (Some(AcceptEdits),Some(AcceptEdits),Plan, AcceptEdits),
            (Some(Autonomous), Some(AcceptEdits),Plan, AcceptEdits),
            (None,             Some(Autonomous), Plan, Autonomous),
            (Some(Plan),       Some(Autonomous), Plan, Plan),
            (Some(AcceptEdits),Some(Autonomous), Plan, AcceptEdits),
            (Some(Autonomous), Some(Autonomous), Plan, Autonomous),
            // default = AcceptEdits
            (None,             None,             AcceptEdits, AcceptEdits),
            (Some(Plan),       None,             AcceptEdits, Plan),
            (Some(AcceptEdits),None,             AcceptEdits, AcceptEdits),
            (Some(Autonomous), None,             AcceptEdits, AcceptEdits),
            (None,             Some(Plan),       AcceptEdits, Plan),
            (Some(Plan),       Some(Plan),       AcceptEdits, Plan),
            (Some(AcceptEdits),Some(Plan),       AcceptEdits, Plan),
            (Some(Autonomous), Some(Plan),       AcceptEdits, Plan),
            (None,             Some(AcceptEdits),AcceptEdits, AcceptEdits),
            (Some(Plan),       Some(AcceptEdits),AcceptEdits, Plan),
            (Some(AcceptEdits),Some(AcceptEdits),AcceptEdits, AcceptEdits),
            (Some(Autonomous), Some(AcceptEdits),AcceptEdits, AcceptEdits),
            (None,             Some(Autonomous), AcceptEdits, Autonomous),
            (Some(Plan),       Some(Autonomous), AcceptEdits, Plan),
            (Some(AcceptEdits),Some(Autonomous), AcceptEdits, AcceptEdits),
            (Some(Autonomous), Some(Autonomous), AcceptEdits, Autonomous),
            // default = Autonomous
            (None,             None,             Autonomous, Autonomous),
            (Some(Plan),       None,             Autonomous, Plan),
            (Some(AcceptEdits),None,             Autonomous, AcceptEdits),
            (Some(Autonomous), None,             Autonomous, Autonomous),
            (None,             Some(Plan),       Autonomous, Plan),
            (Some(Plan),       Some(Plan),       Autonomous, Plan),
            (Some(AcceptEdits),Some(Plan),       Autonomous, Plan),
            (Some(Autonomous), Some(Plan),       Autonomous, Plan),
            (None,             Some(AcceptEdits),Autonomous, AcceptEdits),
            (Some(Plan),       Some(AcceptEdits),Autonomous, Plan),
            (Some(AcceptEdits),Some(AcceptEdits),Autonomous, AcceptEdits),
            (Some(Autonomous), Some(AcceptEdits),Autonomous, AcceptEdits),
            (None,             Some(Autonomous), Autonomous, Autonomous),
            (Some(Plan),       Some(Autonomous), Autonomous, Plan),
            (Some(AcceptEdits),Some(Autonomous), Autonomous, AcceptEdits),
            (Some(Autonomous), Some(Autonomous), Autonomous, Autonomous),
        ];
        assert_eq!(table.len(), 48, "4 ceilings x 4 overrides x 3 defaults");
        for &(ceiling, override_mode, default_mode, expected) in table {
            assert_eq!(
                effective_mode(ceiling, override_mode, default_mode),
                expected,
                "ceiling={ceiling:?} override={override_mode:?} default={default_mode:?}"
            );
        }
    }
}
