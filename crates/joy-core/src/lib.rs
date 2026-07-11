// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

#![deny(clippy::all)]

pub mod agents;
pub mod ai_setup;
pub mod ai_templates;
pub mod app_settings;
pub mod auth;
pub mod capabilities;
pub mod chat_ref;
pub mod chat_state;
pub mod chat_turns;
pub mod chats;
pub mod commit_msg;
pub mod context;
pub mod crypt;
pub mod embedded;
pub mod error;
pub mod event_log;
pub mod filter;
pub mod fortune;
pub mod git_ops;
pub mod guard;
pub mod identity;
pub mod init;
pub mod items;
pub mod jobs;
pub mod member_id;
pub mod member_ref;
pub mod members_file;
pub mod merge;
pub mod migrations;
pub mod milestones;
pub mod model;
pub mod privacy;
pub mod releases;
pub mod security_md;
pub mod store;
pub mod templates;
#[cfg(feature = "tutorial")]
pub mod tutorial;
pub mod vcs;
