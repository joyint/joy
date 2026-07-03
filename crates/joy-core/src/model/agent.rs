// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The AI agent model (git-native, JOY-01EA). An AI member's execution
//! config is `.joy/ai/agents/<member>.yaml`: which tool/adapter runs the
//! member, its model and provider, its default interaction mode, and a
//! default budget. The API key is a SECRET referenced out of band (platform
//! secret store / OS keychain), NEVER written here.

use serde::{Deserialize, Serialize};

use crate::member_ref::MemberRef;
use crate::model::config::InteractionLevel;
use crate::model::job::Budget;

/// Execution configuration for one AI member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    /// The AI member this configures (e.g. ai:claude@joy).
    pub member: MemberRef,
    /// The ACP adapter / tool that runs it (claude-code | mistral-vibe |
    /// qwen-code | mock). `mock` drives the built-out journey without a
    /// real model; the real adapters come at the final step.
    pub adapter: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_mode: Option<InteractionLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_default: Option<Budget>,
}

impl Agent {
    pub fn new(member: MemberRef, adapter: impl Into<String>) -> Self {
        Self {
            member,
            adapter: adapter.into(),
            model: None,
            provider: None,
            default_mode: None,
            budget_default: None,
        }
    }
}
