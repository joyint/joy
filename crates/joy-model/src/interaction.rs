// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! How much oversight a person wants over an AI's actions (JI-0166-D8).

use serde::{Deserialize, Serialize};

/// The three enforceable interaction levels (JI-0166-D8). Declaration order
/// carries the clamp semantics: `Autonomous < Confirmed < Proposing`, where
/// greater means more human oversight. A `max-interaction-level` floor raises
/// the resolved level toward `Proposing`, never lowers it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum InteractionLevel {
    Autonomous,
    Confirmed,
    #[default]
    Proposing,
}

// Hand-written so a pre-2.0 value in persisted YAML surfaces the FromStr
// error pointing at `joy update` instead of serde's generic unknown-variant.
impl<'de> Deserialize<'de> for InteractionLevel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl std::fmt::Display for InteractionLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Autonomous => write!(f, "autonomous"),
            Self::Confirmed => write!(f, "confirmed"),
            Self::Proposing => write!(f, "proposing"),
        }
    }
}

impl std::str::FromStr for InteractionLevel {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "autonomous" => Ok(Self::Autonomous),
            "confirmed" => Ok(Self::Confirmed),
            "proposing" => Ok(Self::Proposing),
            "supervised" | "collaborative" | "interactive" | "pairing" => Err(format!(
                "'{s}' is a pre-2.0 interaction level; run `joy update` to migrate this \
                 repo to the three levels (proposing, confirmed, autonomous)"
            )),
            other => Err(format!("unknown interaction level: {other}")),
        }
    }
}
