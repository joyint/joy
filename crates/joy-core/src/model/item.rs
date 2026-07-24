// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::member_ref::MemberRef;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub item_type: ItemType,
    pub status: Status,
    pub priority: Priority,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assignees: Vec<Assignee>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deps: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<Capability>,
    /// Item-level interaction-level override (the "item" layer of the
    /// resolution in [`super::project::resolve_interaction_level`]).
    #[serde(
        default,
        rename = "interaction-level",
        skip_serializing_if = "Option::is_none"
    )]
    pub interaction_level: Option<super::config::InteractionLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Validity of a decision: where the decided rule currently stands.
    /// Orthogonal to `status`, which tracks the work of deciding. Only
    /// meaningful for `decision` items, and absent while one is still being
    /// decided. A decision binds when `status == closed` and
    /// `validity == accepted` (see the AI tutorial).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validity: Option<Validity>,
    /// For a superseded item: the ID of the item that took its place.
    /// Setting it implies `validity == replaced`. This is a one-way
    /// "replaced by" pointer, distinct from `parent` (containment) and
    /// `deps` (ordering). Used most by decisions, but valid on any item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<MemberRef>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    /// Identity of the last writer of any kind. Stays in sync with `updated`
    /// and serves as a recency hint for sort/UI. `history` carries the
    /// full attribute-change list; this field is the legacy summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<MemberRef>,
    /// Append-only audit list of attribute-level mutations (status, priority,
    /// edit, deps, assignee, milestone, ...). Comment add / edit / rm do NOT
    /// append here. `None` for legacy YAML written before this field existed
    /// (display falls back to `updated` / `updated_by`); `Some(vec![])` for
    /// items created after the field shipped but with no attribute mutations
    /// yet. On first attribute mutation the vec gains its first entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<UpdateEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Name of the Crypt zone this item belongs to. Absent or null
    /// means the item is plaintext. The zone must be declared in the
    /// project's `crypt.zones` registry. See ADR-038 and Crypt.md.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crypt_zone: Option<String>,
    /// Job payload: scope, budget, window and execution attempts. Only
    /// present on `job` items; invisible to every other type. Live
    /// execution telemetry never enters this field -- attempts carry
    /// condensed terminal results only. See JOY-01FE-37.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<JobSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<Comment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemType {
    Epic,
    Story,
    Task,
    Bug,
    Rework,
    Decision,
    Idea,
    /// An assignment of work over a scope of items, executed by an AI or
    /// human assignee. Stored under `.joy/jobs/` (not `.joy/items/`) with
    /// `<ACRONYM>-JOB-xxxx-<hash>` IDs and excluded from default views.
    /// Type-specific data lives in [`Item::job`]. See JOY-01FE-37.
    Job,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Capability {
    // Work capabilities
    Conceive,
    Plan,
    Design,
    Implement,
    Test,
    Review,
    Document,
    /// Direct delegated work: move `job` items through their gates --
    /// approving a job at triage authorizes its spend, reviewing it
    /// accepts the delivered work. Deliberately separate from `Review`
    /// so job acceptance and item acceptance can belong to different
    /// people. See JOY-01FE-37.
    Jobs,
    // Management capabilities
    Create,
    Assign,
    Manage,
    Delete,
}

impl Capability {
    /// All capabilities in canonical order.
    pub const ALL: &[Capability] = &[
        Capability::Conceive,
        Capability::Plan,
        Capability::Design,
        Capability::Implement,
        Capability::Test,
        Capability::Review,
        Capability::Document,
        Capability::Jobs,
        Capability::Create,
        Capability::Assign,
        Capability::Manage,
        Capability::Delete,
    ];

    /// Whether this is a management capability (controls CLI permissions).
    pub fn is_management(&self) -> bool {
        matches!(
            self,
            Capability::Create | Capability::Assign | Capability::Manage | Capability::Delete
        )
    }

    /// Whether this is a work capability (part of the development lifecycle).
    pub fn is_work_capability(&self) -> bool {
        !self.is_management()
    }

    /// All work capabilities in canonical order.
    pub fn work_capabilities() -> Vec<Capability> {
        Self::ALL
            .iter()
            .filter(|c| c.is_work_capability())
            .copied()
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    New,
    Open,
    #[serde(rename = "in-progress")]
    InProgress,
    Review,
    Closed,
    Deferred,
}

/// Where a decision's decided rule currently stands. Orthogonal to
/// [`Status`]: `status` tracks the work of deciding, `Validity` tracks
/// the rule. `proposed` while undecided, `accepted` once it binds,
/// `rejected` if never adopted, `replaced` when another item supersedes
/// it (see [`Item::replaced_by`]), `retired` when dropped without a
/// successor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Validity {
    Proposed,
    Accepted,
    Rejected,
    Replaced,
    Retired,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low,
    Medium,
    High,
    Critical,
    Extreme,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assignee {
    pub member: MemberRef,
    #[serde(rename = "as", default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<Capability>,
}

/// Type-specific payload of a `job` item: what to work on (scope), the
/// limits (budget, window), and the condensed record of execution
/// attempts. Plan data and terminal attempt results are git-native
/// here; live telemetry (progress, running counters) stays with the
/// executor (platform or local runner). See JOY-01FE-37.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobSpec {
    /// Target item IDs. Required at creation -- a job without scope is
    /// just a prompt with a lifecycle. A container reference (epic or
    /// milestone-scoped item) means its subtree; plan jobs reference
    /// the container they create items in.
    pub scope: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<JobBudget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<JobWindow>,
    /// The dialog axis; see [`JobFeedback`]. Absent = no dialog open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<JobFeedback>,
    /// Append-only list of execution loops. Retries and rework rounds
    /// are entries here, not separate objects and not a second status
    /// axis: a job being retried simply stays `in-progress`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempts: Vec<JobAttempt>,
}

/// Spend limits for a job. `max_cents` mirrors the platform's budget
/// unit; `max_tokens` caps raw model usage independently of price.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobBudget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cents: Option<u64>,
    #[serde(default = "default_currency")]
    pub currency: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
}

fn default_currency() -> String {
    "EUR".to_string()
}

/// Execution window for a job: earliest start and latest acceptable end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobWindow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_before: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<DateTime<Utc>>,
}

/// One terminal execution loop of a job. Written once, at the loop's
/// end (success, failure, or abort via `joy stop`), with the cost that
/// actually accrued -- aborts cost money too.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobAttempt {
    pub started: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<DateTime<Utc>>,
    pub outcome: AttemptOutcome,
    #[serde(default)]
    pub tokens: u64,
    #[serde(default)]
    pub cost_cents: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<MemberRef>,
    /// The AI model this attempt ran under (the resolved ai_secrets model,
    /// JI-0164), for cost attribution and audit. None when the adapter's
    /// own default ran or the model was not recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// How an execution loop ended. `Failed` lives here, not in the item
/// status: the job stays `in-progress` while a retry is pending and
/// lands in `review` (carrying the error) when finally given up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttemptOutcome {
    Succeeded,
    Failed,
    Aborted,
}

impl std::fmt::Display for AttemptOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttemptOutcome::Succeeded => write!(f, "succeeded"),
            AttemptOutcome::Failed => write!(f, "failed"),
            AttemptOutcome::Aborted => write!(f, "aborted"),
        }
    }
}

/// Where a job's open dialog currently stands: the dialog axis,
/// orthogonal to [`Status`] exactly like [`Validity`] on decisions.
/// `awaited` = the assignee asked and the operator is up; `received` =
/// the answer is in, the assignee is up; absent = no dialog open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobFeedback {
    Awaited,
    Received,
}

impl std::fmt::Display for JobFeedback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobFeedback::Awaited => write!(f, "awaited"),
            JobFeedback::Received => write!(f, "received"),
        }
    }
}

impl std::str::FromStr for JobFeedback {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "awaited" => Ok(JobFeedback::Awaited),
            "received" => Ok(JobFeedback::Received),
            // "none" clears the field and lives at the CLI layer, like
            // `--validity none`; the enum itself rejects it.
            _ => Err(format!("unknown feedback: {s}")),
        }
    }
}

/// One entry in an item's `history` or a comment's `edits` audit list.
/// Records who touched the artifact and when.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateEntry {
    pub date: DateTime<Utc>,
    pub by: MemberRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    /// Original author. Immutable after creation; comment edits do not
    /// overwrite this field. Editors are recorded in `edits`.
    pub author: MemberRef,
    /// Original creation timestamp. Immutable after creation.
    pub date: DateTime<Utc>,
    pub text: String,
    /// Per-comment edit audit list. Each entry records one `joy comment
    /// edit` invocation: timestamp and editor identity. Append-only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<UpdateEntry>,
}

impl Item {
    pub fn new(
        id: String,
        title: String,
        item_type: ItemType,
        priority: Priority,
        capabilities: Vec<Capability>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id,
            title,
            item_type,
            status: Status::New,
            priority,
            parent: None,
            assignees: Vec::new(),
            deps: Vec::new(),
            milestone: None,
            tags: Vec::new(),
            capabilities,
            interaction_level: None,
            effort: None,
            version: None,
            validity: None,
            replaced_by: None,
            created_by: None,
            created: now,
            updated: now,
            updated_by: None,
            history: Some(Vec::new()),
            description: None,
            crypt_zone: None,
            job: None,
            comments: Vec::new(),
        }
    }

    /// Whether this item is active (not closed or deferred).
    pub fn is_active(&self) -> bool {
        !matches!(self.status, Status::Closed | Status::Deferred)
    }

    /// Whether this item is blocked by any of the given open dependencies.
    pub fn is_blocked_by(&self, items: &[Item]) -> bool {
        if self.deps.is_empty() {
            return false;
        }
        items
            .iter()
            .any(|dep| self.deps.contains(&dep.id) && dep.is_active())
    }
}

impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Capability::Conceive => write!(f, "conceive"),
            Capability::Plan => write!(f, "plan"),
            Capability::Design => write!(f, "design"),
            Capability::Implement => write!(f, "implement"),
            Capability::Test => write!(f, "test"),
            Capability::Review => write!(f, "review"),
            Capability::Document => write!(f, "document"),
            Capability::Jobs => write!(f, "jobs"),
            Capability::Create => write!(f, "create"),
            Capability::Assign => write!(f, "assign"),
            Capability::Manage => write!(f, "manage"),
            Capability::Delete => write!(f, "delete"),
        }
    }
}

impl std::str::FromStr for Capability {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "conceive" | "con" => Ok(Capability::Conceive),
            "plan" | "pln" => Ok(Capability::Plan),
            "design" | "des" => Ok(Capability::Design),
            "implement" | "imp" => Ok(Capability::Implement),
            "test" | "tst" => Ok(Capability::Test),
            "review" | "rev" => Ok(Capability::Review),
            "document" | "doc" => Ok(Capability::Document),
            "jobs" | "job" => Ok(Capability::Jobs),
            "create" | "crt" => Ok(Capability::Create),
            "assign" | "asg" => Ok(Capability::Assign),
            "manage" | "mng" => Ok(Capability::Manage),
            "delete" | "del" => Ok(Capability::Delete),
            _ => Err(format!("unknown capability: {s}")),
        }
    }
}

impl std::fmt::Display for ItemType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ItemType::Epic => write!(f, "epic"),
            ItemType::Story => write!(f, "story"),
            ItemType::Task => write!(f, "task"),
            ItemType::Bug => write!(f, "bug"),
            ItemType::Rework => write!(f, "rework"),
            ItemType::Decision => write!(f, "decision"),
            ItemType::Idea => write!(f, "idea"),
            ItemType::Job => write!(f, "job"),
        }
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Status::New => write!(f, "new"),
            Status::Open => write!(f, "open"),
            Status::InProgress => write!(f, "in-progress"),
            Status::Review => write!(f, "review"),
            Status::Closed => write!(f, "closed"),
            Status::Deferred => write!(f, "deferred"),
        }
    }
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Priority::Low => write!(f, "low"),
            Priority::Medium => write!(f, "medium"),
            Priority::High => write!(f, "high"),
            Priority::Critical => write!(f, "critical"),
            Priority::Extreme => write!(f, "extreme"),
        }
    }
}

impl std::str::FromStr for ItemType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "epic" | "epc" => Ok(ItemType::Epic),
            "story" | "str" => Ok(ItemType::Story),
            "task" | "tsk" => Ok(ItemType::Task),
            "bug" => Ok(ItemType::Bug),
            "rework" | "rwk" => Ok(ItemType::Rework),
            "decision" | "dec" => Ok(ItemType::Decision),
            "idea" | "ide" => Ok(ItemType::Idea),
            "job" => Ok(ItemType::Job),
            _ => Err(format!("unknown item type: {s}")),
        }
    }
}

impl std::str::FromStr for Status {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "new" => Ok(Status::New),
            "open" | "opn" => Ok(Status::Open),
            "in-progress" | "wip" => Ok(Status::InProgress),
            "review" | "rev" => Ok(Status::Review),
            "closed" | "don" => Ok(Status::Closed),
            "deferred" | "def" => Ok(Status::Deferred),
            _ => Err(format!("unknown status: {s}")),
        }
    }
}

impl std::str::FromStr for Priority {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "low" => Ok(Priority::Low),
            "medium" | "med" => Ok(Priority::Medium),
            "high" | "hig" => Ok(Priority::High),
            "critical" | "crt" => Ok(Priority::Critical),
            "extreme" | "ext" => Ok(Priority::Extreme),
            _ => Err(format!("unknown priority: {s}")),
        }
    }
}

impl std::fmt::Display for Validity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Validity::Proposed => write!(f, "proposed"),
            Validity::Accepted => write!(f, "accepted"),
            Validity::Rejected => write!(f, "rejected"),
            Validity::Replaced => write!(f, "replaced"),
            Validity::Retired => write!(f, "retired"),
        }
    }
}

impl std::str::FromStr for Validity {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "proposed" | "pro" => Ok(Validity::Proposed),
            "accepted" | "acc" => Ok(Validity::Accepted),
            "rejected" | "rej" => Ok(Validity::Rejected),
            "replaced" | "rep" => Ok(Validity::Replaced),
            "retired" | "ret" => Ok(Validity::Retired),
            _ => Err(format!("unknown validity: {s}")),
        }
    }
}

/// Generate a slug from a title (lowercase, hyphens, max 40 chars).
pub fn slugify(title: &str) -> String {
    let slug: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    // Collapse multiple hyphens and trim
    let mut result = String::new();
    let mut prev_hyphen = false;
    for c in slug.chars() {
        if c == '-' {
            if !prev_hyphen && !result.is_empty() {
                result.push('-');
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }
    let trimmed = result.trim_end_matches('-');
    if trimmed.len() > 40 {
        // Cut at a char boundary near 40 bytes
        let mut end = 40;
        while end > 0 && !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        let cut = &trimmed[..end];
        let cut = cut.trim_end_matches('-');
        match cut.rfind('-') {
            Some(pos) if pos > 10 => cut[..pos].to_string(),
            _ => cut.to_string(),
        }
    } else {
        trimmed.to_string()
    }
}

/// Build the filename for an item: {ID}-{slug}.yaml
pub fn item_filename(id: &str, title: &str) -> String {
    format!("{}-{}.yaml", id, slugify(title))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_roundtrip() {
        let mut item = Item::new(
            "IT-0001".into(),
            "Login page".into(),
            ItemType::Story,
            Priority::High,
            vec![Capability::Plan, Capability::Implement, Capability::Review],
        );
        item.parent = Some("EP-0001".into());
        item.description = Some("Implement the login page.".into());
        item.tags = vec!["frontend".into()];

        let yaml = serde_yaml_ng::to_string(&item).unwrap();
        let parsed: Item = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(item, parsed);
    }

    #[test]
    fn item_snapshot() {
        use chrono::TimeZone;
        let fixed = Utc.with_ymd_and_hms(2026, 3, 9, 10, 0, 0).unwrap();
        let mut item = Item::new(
            "IT-002A".into(),
            "Payment Integration".into(),
            ItemType::Story,
            Priority::High,
            vec![Capability::Plan, Capability::Implement, Capability::Review],
        );
        item.created = fixed;
        item.updated = fixed;
        item.parent = Some("EP-0001".into());
        item.milestone = Some("MS-01".into());
        item.deps = vec!["IT-0017".into(), "IT-0026".into()];
        item.tags = vec!["backend".into(), "payments".into()];
        item.description =
            Some("Integrate Stripe for payment processing.\nMust support EUR and USD.\n".into());

        let yaml = serde_yaml_ng::to_string(&item).unwrap();
        insta::assert_snapshot!(yaml);
    }

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Payment Integration"), "payment-integration");
    }

    #[test]
    fn slugify_special_chars() {
        assert_eq!(slugify("Fix: crash on Ümlauts!"), "fix-crash-on-ümlauts");
    }

    #[test]
    fn slugify_long_title() {
        let title = "This is a very long title that should be truncated at a reasonable length";
        let slug = slugify(title);
        assert!(slug.len() <= 40);
    }

    #[test]
    fn item_filename_basic() {
        assert_eq!(
            item_filename("IT-0001", "Login page"),
            "IT-0001-login-page.yaml"
        );
    }

    #[test]
    fn is_active_checks() {
        let mut item = Item::new(
            "IT-0001".into(),
            "Test".into(),
            ItemType::Task,
            Priority::Low,
            vec![Capability::Implement],
        );
        assert!(item.is_active());
        item.status = Status::Closed;
        assert!(!item.is_active());
        item.status = Status::Deferred;
        assert!(!item.is_active());
        item.status = Status::InProgress;
        assert!(item.is_active());
    }

    #[test]
    fn parse_item_type() {
        assert_eq!("story".parse::<ItemType>().unwrap(), ItemType::Story);
        assert_eq!("Epic".parse::<ItemType>().unwrap(), ItemType::Epic);
        assert!("unknown".parse::<ItemType>().is_err());
    }

    #[test]
    fn parse_priority() {
        assert_eq!("critical".parse::<Priority>().unwrap(), Priority::Critical);
        assert!("invalid".parse::<Priority>().is_err());
    }

    #[test]
    fn parse_status() {
        assert_eq!("in-progress".parse::<Status>().unwrap(), Status::InProgress);
        assert!("invalid".parse::<Status>().is_err());
    }

    #[test]
    fn parse_validity() {
        assert_eq!("accepted".parse::<Validity>().unwrap(), Validity::Accepted);
        assert_eq!("rep".parse::<Validity>().unwrap(), Validity::Replaced);
        assert_eq!("Retired".parse::<Validity>().unwrap(), Validity::Retired);
        assert!("invalid".parse::<Validity>().is_err());
    }

    #[test]
    fn item_validity_roundtrip() {
        let mut item = Item::new(
            "IT-00CB".into(),
            "Five pillars model".into(),
            ItemType::Decision,
            Priority::High,
            vec![Capability::Conceive, Capability::Plan],
        );
        item.status = Status::Closed;
        item.validity = Some(Validity::Replaced);
        item.replaced_by = Some("IT-0140".into());

        let yaml = serde_yaml_ng::to_string(&item).unwrap();
        let parsed: Item = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(item, parsed);
        // Validity serializes lowercase, like Status.
        assert!(yaml.contains("validity: replaced"));
        assert!(yaml.contains("replaced_by: IT-0140"));
    }

    #[test]
    fn parse_feedback() {
        assert_eq!(
            "awaited".parse::<JobFeedback>().unwrap(),
            JobFeedback::Awaited
        );
        assert_eq!(
            "Received".parse::<JobFeedback>().unwrap(),
            JobFeedback::Received
        );
        // "none" clears the field at the CLI layer; the enum rejects it.
        assert!("none".parse::<JobFeedback>().is_err());
        assert!("invalid".parse::<JobFeedback>().is_err());
    }

    #[test]
    fn job_feedback_roundtrip() {
        let mut item = Item::new(
            "IT-JOB-0001-AA".into(),
            "Deliver the login page".into(),
            ItemType::Job,
            Priority::High,
            vec![Capability::Implement],
        );
        item.job = Some(JobSpec {
            scope: vec!["IT-0001".into()],
            budget: None,
            window: None,
            feedback: Some(JobFeedback::Awaited),
            attempts: Vec::new(),
        });

        let yaml = serde_yaml_ng::to_string(&item).unwrap();
        let parsed: Item = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(item, parsed);
        assert!(yaml.contains("feedback: awaited"));

        // Absent feedback stays off the wire, keeping existing job YAML
        // stable and the field invisible until a dialog opens.
        item.job.as_mut().unwrap().feedback = None;
        let yaml = serde_yaml_ng::to_string(&item).unwrap();
        assert!(!yaml.contains("feedback"));
        let parsed: Item = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(parsed.job.unwrap().feedback, None);
    }
}
