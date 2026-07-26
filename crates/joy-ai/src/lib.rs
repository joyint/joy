// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The AI subsystem for Joy, split out of joy-core per ADR-043. Sits above
//! joy-chat and joy-core.

#![deny(clippy::all)]

pub mod ai_setup;
pub mod ai_templates;
pub mod app_settings;
pub mod chat_turns;
pub mod level_enforcement;
pub mod naming;
pub mod turn_engine;
