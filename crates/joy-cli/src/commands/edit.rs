// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::Args;

use joy_core::guard::Action;
use joy_core::items;
use joy_core::model::item::{
    Capability, Item, ItemType, JobBudget, JobFeedback, JobSpec, JobWindow, Priority, Validity,
};

#[derive(Args)]
pub struct EditArgs {
    /// Item ID (e.g. IT-0001)
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_item_id))]
    id: String,

    /// New title
    #[arg(short, long)]
    title: Option<String>,

    /// Change item type: epic|story|task|bug|rework|decision|idea
    #[arg(short = 'T', long = "type")]
    item_type: Option<String>,

    /// Priority: low|medium|high|critical|extreme
    #[arg(short, long)]
    priority: Option<String>,

    /// Set parent item ID (use "none" to remove)
    #[arg(long)]
    parent: Option<String>,

    /// Effort (1-7, use "none" to remove)
    #[arg(short, long)]
    effort: Option<String>,

    /// New description
    #[arg(short, long)]
    description: Option<String>,

    /// Set milestone (use "none" to remove)
    #[arg(short = 'm', long)]
    milestone: Option<String>,

    /// Tags (comma-separated, replaces existing)
    #[arg(long)]
    tags: Option<String>,

    /// Dependencies (CSV; replaces existing)
    #[arg(long)]
    deps: Option<String>,

    /// Set assignee ("none" clears all)
    #[arg(short = 'A', long)]
    assignee: Option<String>,

    /// Set version tag (use "none" to remove)
    #[arg(short = 'v', long)]
    version: Option<String>,

    /// Decision validity: proposed|accepted|rejected|replaced|retired (use "none" to remove)
    #[arg(long)]
    validity: Option<String>,

    /// ID of the item that replaces this one (use "none" to remove); implies validity=replaced
    #[arg(long = "replaced-by")]
    replaced_by: Option<String>,

    /// Capabilities (CSV; replaces existing)
    #[arg(short = 'c', long)]
    capabilities: Option<String>,

    /// Job scope (CSV replaces; +ID/-ID entries add/remove)
    #[arg(long, allow_hyphen_values = true)]
    scope: Option<String>,

    /// Job budget: maximum cost as a decimal, e.g. 12.50
    #[arg(long = "max-cost")]
    max_cost: Option<String>,

    /// Job budget: maximum model tokens
    #[arg(long = "max-tokens")]
    max_tokens: Option<u64>,

    /// Job window: earliest start (YYYY-MM-DD or RFC3339)
    #[arg(long = "not-before")]
    not_before: Option<String>,

    /// Job window: latest acceptable end (YYYY-MM-DD or RFC3339)
    #[arg(long)]
    deadline: Option<String>,

    /// Job dialog state: awaited|received (use "none" to remove)
    #[arg(long)]
    feedback: Option<String>,
}

pub fn run(args: EditArgs) -> Result<()> {
    let ctx = crate::crypt_session::load_context(None)?;

    let mut item = items::load_item(&ctx.root, &args.id)?;
    let mut changed = false;

    if let Some(title) = args.title {
        item.title = title;
        changed = true;
    }

    if let Some(ref t) = args.item_type {
        item.item_type = t
            .parse::<ItemType>()
            .map_err(|e: String| anyhow::anyhow!("{}", e))?;
        changed = true;
    }

    if let Some(ref p) = args.priority {
        item.priority = p
            .parse::<Priority>()
            .map_err(|e: String| anyhow::anyhow!("{}", e))?;
        changed = true;
    }

    if let Some(ref effort_str) = args.effort {
        item.effort = crate::effort::parse_effort(effort_str)?;
        changed = true;
    }

    if let Some(ref parent) = args.parent {
        if parent == "none" {
            item.parent = None;
        } else {
            match items::load_item(&ctx.root, parent) {
                Ok(parent_item) => {
                    if !parent_item.is_active() {
                        eprintln!("Warning: parent {} is {}.", parent, parent_item.status);
                    }
                }
                Err(_) => {
                    if parent.contains("-MS-") {
                        anyhow::bail!("{} is a milestone, not an item. Use `joy milestone link {} {}` instead.", parent, item.id, parent);
                    }
                    anyhow::bail!("parent {} is not a valid item ID.", parent);
                }
            }
            item.parent = Some(parent.clone());
        }
        changed = true;
    }

    if let Some(desc) = args.description {
        item.description = Some(desc);
        changed = true;
    }

    if let Some(ref ms) = args.milestone {
        item.milestone = if ms == "none" { None } else { Some(ms.clone()) };
        changed = true;
    }

    if let Some(ref tags) = args.tags {
        item.tags = if tags.is_empty() {
            Vec::new()
        } else {
            tags.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };
        changed = true;
    }

    if let Some(ref deps) = args.deps {
        if deps.is_empty() {
            item.deps = Vec::new();
        } else {
            let new_deps: Vec<String> = deps.split(',').map(|s| s.trim().to_string()).collect();
            for dep_id in &new_deps {
                if let Some(cycle) = items::detect_cycle(&ctx.root, &item.id, dep_id)? {
                    anyhow::bail!("circular dependency: {}", cycle.join(" -> "));
                }
            }
            item.deps = new_deps;
        }
        changed = true;
    }

    if let Some(ref version) = args.version {
        item.version = if version == "none" {
            None
        } else {
            Some(version.clone())
        };
        changed = true;
    }

    if let Some(ref validity) = args.validity {
        item.validity = if validity == "none" {
            None
        } else {
            Some(
                validity
                    .parse::<Validity>()
                    .map_err(|e: String| anyhow::anyhow!("{}", e))?,
            )
        };
        changed = true;
    }

    if let Some(ref replaced) = args.replaced_by {
        if replaced == "none" {
            item.replaced_by = None;
        } else {
            if replaced == &item.id {
                anyhow::bail!("an item cannot be replaced by itself.");
            }
            if items::load_item(&ctx.root, replaced).is_err() {
                anyhow::bail!("replaced_by {} is not a valid item ID.", replaced);
            }
            item.replaced_by = Some(replaced.clone());
            // A successor implies this item is no longer the current one.
            item.validity = Some(Validity::Replaced);
        }
        changed = true;
    }

    if let Some(ref caps) = args.capabilities {
        if caps.is_empty() {
            item.capabilities = Vec::new();
        } else {
            item.capabilities = caps
                .split(',')
                .map(|s| {
                    s.trim()
                        .parse::<Capability>()
                        .map_err(|e| anyhow::anyhow!("{}", e))
                })
                .collect::<Result<Vec<_>>>()?;
        }
        changed = true;
    }

    let job_flags = args.scope.is_some()
        || args.max_cost.is_some()
        || args.max_tokens.is_some()
        || args.not_before.is_some()
        || args.deadline.is_some()
        || args.feedback.is_some();
    if job_flags && !matches!(item.item_type, ItemType::Job) {
        anyhow::bail!(
            "--scope, --max-cost, --max-tokens, --not-before, --deadline and --feedback are only valid for job items"
        );
    }

    if let Some(ref spec) = args.scope {
        let current = item
            .job
            .as_ref()
            .map(|j| j.scope.clone())
            .unwrap_or_default();
        let scope = apply_scope_spec(&ctx.root, &current, spec)?;
        item.job.get_or_insert_with(empty_job_spec).scope = scope;
        changed = true;
    }

    if let Some(ref cost) = args.max_cost {
        let cents =
            parse_decimal_cents(cost).map_err(|e| anyhow::anyhow!("invalid --max-cost: {}", e))?;
        job_budget(&mut item).max_cents = Some(cents);
        changed = true;
    }

    if let Some(tokens) = args.max_tokens {
        job_budget(&mut item).max_tokens = Some(tokens);
        changed = true;
    }

    if let Some(ref when) = args.not_before {
        job_window(&mut item).not_before = Some(parse_when(when, "--not-before")?);
        changed = true;
    }

    if let Some(ref when) = args.deadline {
        job_window(&mut item).deadline = Some(parse_when(when, "--deadline")?);
        changed = true;
    }

    if let Some(ref feedback) = args.feedback {
        if feedback == "none" {
            // Closing a dialog on a job without a spec is a no-op; do
            // not materialize an empty spec just to hold a None.
            if let Some(job) = item.job.as_mut() {
                job.feedback = None;
            }
        } else {
            let parsed = feedback
                .parse::<JobFeedback>()
                .map_err(|e: String| anyhow::anyhow!("{}", e))?;
            item.job.get_or_insert_with(empty_job_spec).feedback = Some(parsed);
        }
        changed = true;
    }

    if let Some(ref assignee) = args.assignee {
        if assignee == "none" {
            item.assignees.clear();
        } else {
            // Simple single-assignee via edit: replaces all assignees
            item.assignees = vec![joy_core::model::item::Assignee {
                member: assignee.clone().into(),
                capabilities: Vec::new(),
            }];
        }
        changed = true;
    }

    if !changed {
        if crate::output::is_json() {
            return crate::output::emit(&item);
        }
        println!("Nothing to change. Use flags like --title, --priority, --parent, etc.");
        return Ok(());
    }

    // Guard check: AssignItem if assignee changed, UpdateItem otherwise
    let action = if args.assignee.is_some() {
        Action::AssignItem
    } else {
        Action::UpdateItem
    };
    ctx.enforce(&action, &item.id)?;

    let log_user = ctx.log_user();
    items::touch_for_attribute_change(&mut item, &log_user);
    items::update_item(&ctx.root, &item)?;
    joy_core::event_log::log_event_as(
        &ctx.root,
        joy_core::event_log::EventType::ItemUpdated,
        &item.id,
        None,
        &log_user,
    );

    if crate::output::is_json() {
        crate::output::emit(&item)?;
    } else {
        println!("Updated {} {}", item.id, item.title);
    }

    joy_core::git_ops::auto_git_post_command(
        &ctx.root,
        &format!("edit {} {}", item.id, item.title),
        &log_user,
    );

    Ok(())
}

fn empty_job_spec() -> JobSpec {
    JobSpec {
        scope: Vec::new(),
        budget: None,
        window: None,
        feedback: None,
        attempts: Vec::new(),
    }
}

fn job_budget(item: &mut Item) -> &mut JobBudget {
    item.job
        .get_or_insert_with(empty_job_spec)
        .budget
        .get_or_insert_with(|| JobBudget {
            max_cents: None,
            currency: "EUR".to_string(),
            max_tokens: None,
        })
}

fn job_window(item: &mut Item) -> &mut JobWindow {
    item.job
        .get_or_insert_with(empty_job_spec)
        .window
        .get_or_insert_with(|| JobWindow {
            not_before: None,
            deadline: None,
        })
}

/// Apply a `--scope` spec to the current scope list. The spec is either
/// a plain comma list (replace) or +ID/-ID entries (add/remove); mixing
/// the two forms is rejected.
fn apply_scope_spec(root: &std::path::Path, current: &[String], spec: &str) -> Result<Vec<String>> {
    let entries: Vec<&str> = spec
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if entries.is_empty() {
        anyhow::bail!("a job needs at least one scope item");
    }

    let delta = entries
        .iter()
        .any(|e| e.starts_with('+') || e.starts_with('-'));
    let mut scope: Vec<String>;
    if delta {
        if !entries
            .iter()
            .all(|e| e.starts_with('+') || e.starts_with('-'))
        {
            anyhow::bail!("--scope cannot mix +/- entries with plain IDs; use one form");
        }
        scope = current.to_vec();
        for entry in &entries {
            let (op, sid) = entry.split_at(1);
            let sid = sid.trim();
            if op == "+" {
                let full = resolve_scope_item(root, sid)?;
                if !scope.contains(&full) {
                    scope.push(full);
                }
            } else {
                // Normalize a short form when it still resolves; a stale
                // ID that no longer loads is matched verbatim.
                let full = items::load_item(root, sid)
                    .map(|i| i.id)
                    .unwrap_or_else(|_| sid.to_string());
                let before = scope.len();
                scope.retain(|s| s != &full && s != sid);
                if scope.len() == before {
                    anyhow::bail!("{} is not in the scope of this job", sid);
                }
            }
        }
    } else {
        scope = Vec::new();
        for sid in &entries {
            let full = resolve_scope_item(root, sid)?;
            if !scope.contains(&full) {
                scope.push(full);
            }
        }
    }

    if scope.is_empty() {
        anyhow::bail!("a job needs at least one scope item");
    }
    Ok(scope)
}

/// Validate one scope addition: must resolve to an existing non-job
/// item; returns the full (normalized) item ID.
fn resolve_scope_item(root: &std::path::Path, sid: &str) -> Result<String> {
    if items::is_job_id(sid) {
        anyhow::bail!("a job cannot scope another job; use deps for job ordering");
    }
    let scope_item = items::load_item(root, sid)
        .map_err(|_| anyhow::anyhow!("scope item {} is not a valid item ID.", sid))?;
    if scope_item.item_type == ItemType::Job {
        anyhow::bail!("a job cannot scope another job; use deps for job ordering");
    }
    Ok(scope_item.id)
}

/// Parse a decimal money amount ("12", "12.5", "12.50") into cents.
fn parse_decimal_cents(s: &str) -> Result<u64> {
    let err = || anyhow::anyhow!("'{}' is not a decimal amount like 12.50", s);
    let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
    if whole.is_empty()
        || frac.len() > 2
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !frac.chars().all(|c| c.is_ascii_digit())
    {
        return Err(err());
    }
    let whole: u64 = whole.parse().map_err(|_| err())?;
    let frac_cents = match frac.len() {
        0 => 0,
        1 => frac.parse::<u64>().map_err(|_| err())? * 10,
        _ => frac.parse::<u64>().map_err(|_| err())?,
    };
    Ok(whole * 100 + frac_cents)
}

/// Parse `YYYY-MM-DD` (midnight UTC) or a full RFC3339 timestamp.
fn parse_when(s: &str, flag: &str) -> Result<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        if let Some(dt) = date.and_hms_opt(0, 0, 0) {
            return Ok(DateTime::from_naive_utc_and_offset(dt, Utc));
        }
    }
    anyhow::bail!("invalid {} '{}': expected YYYY-MM-DD or RFC3339", flag, s)
}

#[cfg(test)]
mod tests {
    use super::parse_decimal_cents;

    #[test]
    fn decimal_cents_accepts_common_forms() {
        assert_eq!(parse_decimal_cents("12").unwrap(), 1200);
        assert_eq!(parse_decimal_cents("12.5").unwrap(), 1250);
        assert_eq!(parse_decimal_cents("12.50").unwrap(), 1250);
        assert_eq!(parse_decimal_cents("0.07").unwrap(), 7);
    }

    #[test]
    fn decimal_cents_rejects_garbage() {
        assert!(parse_decimal_cents("").is_err());
        assert!(parse_decimal_cents(".50").is_err());
        assert!(parse_decimal_cents("12.505").is_err());
        assert!(parse_decimal_cents("-3").is_err());
        assert!(parse_decimal_cents("12,50").is_err());
    }
}
