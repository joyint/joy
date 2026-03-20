// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deps: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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
pub struct Comment {
    pub author: String,
    pub date: DateTime<Utc>,
    pub text: String,
}

impl Item {
    pub fn new(id: String, title: String, item_type: ItemType, priority: Priority) -> Self {
        let now = Utc::now();
        Self {
            id,
            title,
            item_type,
            status: Status::New,
            priority,
            parent: None,
            assignee: None,
            deps: Vec::new(),
            milestone: None,
            tags: Vec::new(),
            size: None,
            version: None,
            created: now,
            updated: now,
            description: None,
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
            "epic" => Ok(ItemType::Epic),
            "story" => Ok(ItemType::Story),
            "task" => Ok(ItemType::Task),
            "bug" => Ok(ItemType::Bug),
            "rework" => Ok(ItemType::Rework),
            "decision" => Ok(ItemType::Decision),
            "idea" => Ok(ItemType::Idea),
            _ => Err(format!("unknown item type: {s}")),
        }
    }
}

impl std::str::FromStr for Status {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "new" => Ok(Status::New),
            "open" => Ok(Status::Open),
            "in-progress" => Ok(Status::InProgress),
            "review" => Ok(Status::Review),
            "closed" => Ok(Status::Closed),
            "deferred" => Ok(Status::Deferred),
            _ => Err(format!("unknown status: {s}")),
        }
    }
}

impl std::str::FromStr for Priority {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "low" => Ok(Priority::Low),
            "medium" => Ok(Priority::Medium),
            "high" => Ok(Priority::High),
            "critical" => Ok(Priority::Critical),
            "extreme" => Ok(Priority::Extreme),
            _ => Err(format!("unknown priority: {s}")),
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
        );
        item.parent = Some("EP-0001".into());
        item.description = Some("Implement the login page.".into());
        item.tags = vec!["frontend".into()];

        let yaml = serde_yml::to_string(&item).unwrap();
        let parsed: Item = serde_yml::from_str(&yaml).unwrap();
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
        );
        item.created = fixed;
        item.updated = fixed;
        item.parent = Some("EP-0001".into());
        item.milestone = Some("MS-01".into());
        item.deps = vec!["IT-0017".into(), "IT-0026".into()];
        item.tags = vec!["backend".into(), "payments".into()];
        item.description =
            Some("Integrate Stripe for payment processing.\nMust support EUR and USD.\n".into());

        let yaml = serde_yml::to_string(&item).unwrap();
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
}
