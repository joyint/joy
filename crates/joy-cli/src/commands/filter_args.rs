// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Shared filter flags for listing views (ls, board, roadmap).
//!
//! Defined once and embedded into each view's argument struct via
//! `#[command(flatten)]`. The CLI-side struct here is the only place
//! that knows about clap; converting to a [`FilterSpec`] crosses into
//! the CLI-free filter implementation in joy-core.

use std::path::Path;

use anyhow::{anyhow, bail, Result};
use clap::Args;

use joy_core::filter::{FilterSpec, MemberFilter};
use joy_core::identity;
use joy_core::model::item::{ItemType, Priority, Status};
use joy_core::store;

/// Reserved tokens that the `--members` filter recognises in addition to
/// literal member IDs.
const TOKEN_ME: &str = "me";
const TOKEN_NONE: &str = "none";
const TOKEN_UNASSIGNED: &str = "unassigned";
const TOKEN_ANY: &str = "*";

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

    /// Filter by member (comma-separated). Tokens: 'me' = current user,
    /// 'none'/'unassigned' = items without assignees, '*' = items with
    /// any assignee.
    #[arg(
        short = 'm',
        long,
        value_delimiter = ',',
        add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_member),
    )]
    pub members: Vec<String>,

    /// Show only items assigned to me. Equivalent to `--members me`.
    #[arg(long)]
    pub mine: bool,

    /// Filter by milestone ID (includes items inheriting from parent)
    #[arg(short = 'M', long)]
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
}

impl FilterArgs {
    /// Resolve string-typed flags into a [`FilterSpec`] usable by joy-core.
    /// `include_closed` controls the FilterSpec.all field: views with
    /// different "show closed by default" policies pass it themselves.
    /// `root` is needed only when `--mine` or `me` triggers identity
    /// resolution and AI-delegation expansion.
    pub fn to_spec(&self, root: &Path, include_closed: bool) -> Result<FilterSpec> {
        let item_type: Option<ItemType> = self
            .item_type
            .as_deref()
            .map(|t| t.parse().map_err(|e: String| anyhow!("{}", e)))
            .transpose()?;

        let status: Option<Status> = self
            .status
            .as_deref()
            .map(|s| s.parse().map_err(|e: String| anyhow!("{}", e)))
            .transpose()?;

        let priority: Option<Priority> = self
            .priority
            .as_deref()
            .map(|p| p.parse().map_err(|e: String| anyhow!("{}", e)))
            .transpose()?;

        let members = resolve_members(&self.members, self.mine, root)?;

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
            all: include_closed,
        })
    }
}

/// Parse the `--members` token list and the `--mine` flag into a
/// `MemberFilter`. `'none'` / `'unassigned'` and `'*'` are exclusive
/// modes and reject mixing with any other token. `'me'` and `--mine`
/// expand to the current human's member ID plus any AI member ID
/// they have a delegation entry for.
fn resolve_members(tokens: &[String], mine: bool, root: &Path) -> Result<MemberFilter> {
    let mut wants_self = mine;
    let mut wants_unassigned = false;
    let mut wants_any_assigned = false;
    let mut explicit: Vec<String> = Vec::new();

    for raw in tokens {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        match token.to_ascii_lowercase().as_str() {
            TOKEN_ME => wants_self = true,
            TOKEN_NONE | TOKEN_UNASSIGNED => wants_unassigned = true,
            t if t == TOKEN_ANY => wants_any_assigned = true,
            _ => explicit.push(token.to_string()),
        }
    }

    let exclusive_count = [wants_unassigned, wants_any_assigned]
        .iter()
        .filter(|b| **b)
        .count();
    if exclusive_count > 1 {
        bail!("--members 'none' and '*' are mutually exclusive");
    }
    if (wants_unassigned || wants_any_assigned) && (wants_self || !explicit.is_empty()) {
        bail!("--members 'none'/'unassigned' and '*' cannot be combined with member IDs or 'me'");
    }

    if wants_unassigned {
        return Ok(MemberFilter::Unassigned);
    }
    if wants_any_assigned {
        return Ok(MemberFilter::AnyAssigned);
    }

    let mut resolved: Vec<String> = explicit;
    if wants_self {
        for id in resolve_self_with_delegated_ais(root)? {
            if !resolved.contains(&id) {
                resolved.push(id);
            }
        }
    }

    if resolved.is_empty() {
        Ok(MemberFilter::Any)
    } else {
        Ok(MemberFilter::Specific(resolved))
    }
}

/// Expand "me" symmetrically across the (human, AI) delegation pair.
/// Whether the session is human or AI, the resulting set always
/// contains: the human's own member ID, the AI's member ID (if any),
/// and any sibling AI the human has delegated to.
fn resolve_self_with_delegated_ais(root: &Path) -> Result<Vec<String>> {
    let identity = identity::resolve_identity(root).map_err(|e| anyhow!("{e}"))?;
    let mut members: Vec<String> = vec![identity.member.clone()];

    let human_id = identity
        .delegated_by
        .clone()
        .unwrap_or_else(|| identity.member.clone());

    if !members.contains(&human_id) {
        members.push(human_id.clone());
    }

    if let Ok(project) = store::load_project(root) {
        if let Some(member) = project.members.get(&human_id) {
            for ai_id in member.ai_delegations.keys() {
                if !members.contains(ai_id) {
                    members.push(ai_id.clone());
                }
            }
        }
    }

    Ok(members)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn parse(tokens: &[&str], mine: bool) -> Result<MemberFilter> {
        // Tests bypass identity resolution by avoiding `me` / `--mine`.
        let owned: Vec<String> = tokens.iter().map(|s| (*s).to_string()).collect();
        resolve_members(&owned, mine, &PathBuf::from("/nonexistent"))
    }

    #[test]
    fn empty_yields_any() {
        assert_eq!(parse(&[], false).unwrap(), MemberFilter::Any);
    }

    #[test]
    fn explicit_ids_yield_specific() {
        assert_eq!(
            parse(&["alice@x.com", "bob@x.com"], false).unwrap(),
            MemberFilter::Specific(vec!["alice@x.com".into(), "bob@x.com".into()])
        );
    }

    #[test]
    fn none_yields_unassigned() {
        assert_eq!(parse(&["none"], false).unwrap(), MemberFilter::Unassigned);
        assert_eq!(
            parse(&["unassigned"], false).unwrap(),
            MemberFilter::Unassigned
        );
    }

    #[test]
    fn star_yields_any_assigned() {
        assert_eq!(parse(&["*"], false).unwrap(), MemberFilter::AnyAssigned);
    }

    #[test]
    fn special_tokens_case_insensitive() {
        assert_eq!(parse(&["NONE"], false).unwrap(), MemberFilter::Unassigned);
        assert_eq!(parse(&["None"], false).unwrap(), MemberFilter::Unassigned);
    }

    #[test]
    fn whitespace_tokens_are_trimmed() {
        assert_eq!(parse(&[" ", ""], false).unwrap(), MemberFilter::Any);
    }

    #[test]
    fn none_combined_with_id_errors() {
        let err = parse(&["none", "alice@x.com"], false).unwrap_err();
        assert!(err.to_string().contains("cannot be combined"));
    }

    #[test]
    fn star_combined_with_mine_errors() {
        let err = parse(&["*"], true).unwrap_err();
        assert!(err.to_string().contains("cannot be combined"));
    }

    #[test]
    fn none_and_star_together_error() {
        let err = parse(&["none", "*"], false).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"));
    }
}
