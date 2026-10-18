// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The AI subsystem for Joy, split out of joy-core per ADR-043. Sits above
//! joy-chat and joy-core.

#![deny(clippy::all)]

// Nothing of the developer's shell and session reaches the unit tests
// of this crate (JOY-02BB-C7).
#[cfg(test)]
joy_test_env::isolate!();

#[cfg(feature = "acp")]
pub mod acp_lane;
pub mod activity;
#[cfg(feature = "acp")]
pub mod adapter_behaviour;
pub mod adapters;
pub mod ai_setup;
pub mod ai_templates;
pub mod chat_turns;
pub mod level_enforcement;
pub mod naming;
pub mod turn_engine;
