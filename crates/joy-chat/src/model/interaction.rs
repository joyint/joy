// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Interaction-level resolution for chat overrides (JI-0166-D8 §5,
//! JOY-0228-8C) and the read-compat for pre-2.0 chat state.
//!
//! The persisted per-(chat, AI, delegator) override is an
//! [`InteractionLevel`]; the [`AgentMode`](super::agent_mode::AgentMode)
//! a turn runs under is derived one-way from the effective level at the
//! ACP boundary and never stored.
//!
//! Read-compat: sealed chat blobs are encrypted and replicated, so they
//! cannot be batch-rewritten by a `joy update` reconcile the way repo
//! YAML is. Instead, deserialization of persisted chat state accepts the
//! pre-2.0 agent-mode names (`plan` -> `proposing`, `accept-edits` ->
//! `confirmed`; `autonomous` already parses as a level) and the next
//! persist writes only level names. This leniency exists ONLY on the
//! chat-state read path; everything else uses the strict joy-core parse.

use joy_model::InteractionLevel;
use serde::Deserialize;
use std::collections::BTreeMap;

/// Resolve the level a turn actually runs under (JI-0166-D8 §5): the
/// per-chat, per-delegator override when one is stored (else the
/// member's default level), clamped by the project floor when one is
/// set.
///
/// Pure lattice math on [`InteractionLevel`]'s `Ord` (greater = more
/// human oversight): `max(override or default, floor or Autonomous)`.
pub fn effective_level(
    floor: Option<InteractionLevel>,
    override_level: Option<InteractionLevel>,
    default_level: InteractionLevel,
) -> InteractionLevel {
    let chosen = override_level.unwrap_or(default_level);
    floor.map_or(chosen, |f| chosen.max(f))
}

/// Parse a persisted level value, accepting the pre-2.0 agent-mode
/// names. `None` for anything unknown.
pub fn parse_level_compat(s: &str) -> Option<InteractionLevel> {
    match s {
        "plan" => Some(InteractionLevel::Proposing),
        "accept-edits" => Some(InteractionLevel::Confirmed),
        other => other.parse().ok(),
    }
}

/// `deserialize_with` helper for a single persisted level value.
pub(crate) fn de_level_compat<'de, D>(deserializer: D) -> Result<InteractionLevel, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    parse_level_compat(&s)
        .ok_or_else(|| serde::de::Error::custom(format!("unknown interaction level: {s}")))
}

/// `deserialize_with` helper for the nested override map
/// (AI participant id -> delegator id -> level).
pub(crate) fn de_level_nested_map<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, BTreeMap<String, InteractionLevel>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let raw: BTreeMap<String, BTreeMap<String, String>> = Deserialize::deserialize(deserializer)?;
    raw.into_iter()
        .map(|(agent, per_delegator)| {
            let per_delegator = per_delegator
                .into_iter()
                .map(|(delegator, value)| {
                    parse_level_compat(&value)
                        .ok_or_else(|| {
                            serde::de::Error::custom(format!("unknown interaction level: {value}"))
                        })
                        .map(|level| (delegator, level))
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?;
            Ok((agent, per_delegator))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use joy_model::InteractionLevel::{Autonomous, Confirmed, Proposing};

    #[test]
    fn lattice_ordering_matches_oversight() {
        // Greater = more oversight; a floor raises toward Proposing.
        assert!(Autonomous < Confirmed);
        assert!(Confirmed < Proposing);
    }

    #[test]
    fn effective_level_truth_table() {
        // (floor, override, default) -> effective, hand-computed as
        // max(override or default, floor or Autonomous).
        #[rustfmt::skip]
        let table: &[(Option<InteractionLevel>, Option<InteractionLevel>, InteractionLevel, InteractionLevel)] = &[
            (None,             None,            Autonomous, Autonomous),
            (None,             None,            Proposing,  Proposing),
            (None,             Some(Autonomous),Proposing,  Autonomous),
            (None,             Some(Proposing), Autonomous, Proposing),
            (Some(Confirmed),  None,            Autonomous, Confirmed),
            (Some(Confirmed),  None,            Proposing,  Proposing),
            (Some(Confirmed),  Some(Autonomous),Proposing,  Confirmed),
            (Some(Proposing),  Some(Autonomous),Autonomous, Proposing),
            (Some(Autonomous), Some(Confirmed), Autonomous, Confirmed),
        ];
        for &(floor, override_level, default_level, expected) in table {
            assert_eq!(
                effective_level(floor, override_level, default_level),
                expected,
                "floor={floor:?} override={override_level:?} default={default_level:?}"
            );
        }
    }

    #[test]
    fn compat_parses_agent_mode_names_and_levels() {
        assert_eq!(parse_level_compat("plan"), Some(Proposing));
        assert_eq!(parse_level_compat("accept-edits"), Some(Confirmed));
        assert_eq!(parse_level_compat("autonomous"), Some(Autonomous));
        assert_eq!(parse_level_compat("proposing"), Some(Proposing));
        assert_eq!(parse_level_compat("confirmed"), Some(Confirmed));
        assert_eq!(parse_level_compat("yolo"), None);
        // Pre-2.0 five-level names never existed in chat overrides and
        // stay rejected.
        assert_eq!(parse_level_compat("pairing"), None);
    }
}
