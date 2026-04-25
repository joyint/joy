// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Shared filter flags for listing views (ls, board, roadmap).
//!
//! Defined once and embedded into each view's argument struct via
//! `#[command(flatten)]`. The CLI-side struct here is the only place
//! that knows about clap; converting to a [`FilterSpec`] crosses into
//! the CLI-free filter implementation in joy-core.

use anyhow::Result;
use clap::Args;

use joy_core::filter::FilterSpec;
use joy_core::model::item::{ItemType, Priority, Status};
use joy_core::vcs::Vcs;

#[derive(Args, Default, Clone)]
pub struct FilterArgs {
    /// Filter by ancestor item ID (shows the item and all descendants)
    #[arg(long)]
    pub parent: Option<String>,

    /// Filter by type: epic, story, task, bug, rework, decision, idea
    #[arg(short = 'T', long = "type")]
    pub item_type: Option<String>,

    /// Filter by status: new, open, in-progress, review, closed, deferred
    #[arg(short, long)]
    pub status: Option<String>,

    /// Filter by priority: low, medium, high, critical, extreme
    #[arg(short, long)]
    pub priority: Option<String>,

    /// Show only items assigned to me (git config user.email)
    #[arg(short = 'M', long)]
    pub mine: bool,

    /// Filter by milestone ID (includes items inheriting from parent)
    #[arg(short, long)]
    pub milestone: Option<String>,

    /// Filter by tag
    #[arg(long)]
    pub tag: Option<String>,

    /// Filter by version tag
    #[arg(short = 'v', long)]
    pub version: Option<String>,

    /// Show only blocked items
    #[arg(short, long)]
    pub blocked: bool,

    /// Show all items (including closed and deferred)
    #[arg(short, long)]
    pub all: bool,
}

impl FilterArgs {
    /// Resolve string-typed flags into a [`FilterSpec`] usable by joy-core.
    /// `--mine` is resolved via the configured VCS user email; AI members
    /// authenticated via JOY_SESSION are not yet considered (covered by
    /// the dedicated members filter introduced in JOY-0115-8B).
    pub fn to_spec(&self) -> Result<FilterSpec> {
        let item_type: Option<ItemType> = self
            .item_type
            .as_deref()
            .map(|t| t.parse().map_err(|e: String| anyhow::anyhow!("{}", e)))
            .transpose()?;

        let status: Option<Status> = self
            .status
            .as_deref()
            .map(|s| s.parse().map_err(|e: String| anyhow::anyhow!("{}", e)))
            .transpose()?;

        let priority: Option<Priority> = self
            .priority
            .as_deref()
            .map(|p| p.parse().map_err(|e: String| anyhow::anyhow!("{}", e)))
            .transpose()?;

        let members: Vec<String> = if self.mine {
            vec![joy_core::vcs::default_vcs()
                .user_email()
                .map_err(|e| anyhow::anyhow!("{e}"))?]
        } else {
            Vec::new()
        };

        Ok(FilterSpec {
            parent: self.parent.clone(),
            item_type,
            status,
            priority,
            milestone: self.milestone.clone(),
            tag: self.tag.clone(),
            version: self.version.clone(),
            members,
            blocked: self.blocked,
            all: self.all,
        })
    }
}
