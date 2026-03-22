// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// A contributor to a release with item count.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Contributor {
    pub id: String,
    pub events: usize,
    pub items: usize,
}

/// A released item reference (ID + title snapshot).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReleaseItem {
    pub id: String,
    pub title: String,
}

/// Items grouped by type within a release.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReleaseItems {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub epics: Vec<ReleaseItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stories: Vec<ReleaseItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<ReleaseItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bugs: Vec<ReleaseItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reworks: Vec<ReleaseItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<ReleaseItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ideas: Vec<ReleaseItem>,
}

impl ReleaseItems {
    pub fn is_empty(&self) -> bool {
        self.epics.is_empty()
            && self.stories.is_empty()
            && self.tasks.is_empty()
            && self.bugs.is_empty()
            && self.reworks.is_empty()
            && self.decisions.is_empty()
            && self.ideas.is_empty()
    }

    pub fn total(&self) -> usize {
        self.epics.len()
            + self.stories.len()
            + self.tasks.len()
            + self.bugs.len()
            + self.reworks.len()
            + self.decisions.len()
            + self.ideas.len()
    }
}

/// A release snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub date: NaiveDate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contributors: Vec<Contributor>,
    pub items: ReleaseItems,
}

/// Compute the next semver version from a current version string.
pub fn bump_version(current: &str, bump: Bump) -> String {
    let v = current.strip_prefix('v').unwrap_or(current);
    let parts: Vec<&str> = v.splitn(3, '.').collect();
    let major: u32 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor: u32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let patch: u32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

    match bump {
        Bump::Major => format!("v{}.0.0", major + 1),
        Bump::Minor => format!("v{}.{}.0", major, minor + 1),
        Bump::Patch => format!("v{}.{}.{}", major, minor, patch + 1),
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Bump {
    Major,
    Minor,
    Patch,
}

impl std::str::FromStr for Bump {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "major" => Ok(Self::Major),
            "minor" => Ok(Self::Minor),
            "patch" => Ok(Self::Patch),
            _ => Err(format!("invalid bump: {s} (use major, minor, or patch)")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_patch() {
        assert_eq!(bump_version("v0.3.1", Bump::Patch), "v0.3.2");
    }

    #[test]
    fn bump_minor() {
        assert_eq!(bump_version("v0.3.1", Bump::Minor), "v0.4.0");
    }

    #[test]
    fn bump_major() {
        assert_eq!(bump_version("v0.3.1", Bump::Major), "v1.0.0");
    }

    #[test]
    fn bump_without_prefix() {
        assert_eq!(bump_version("1.2.3", Bump::Patch), "v1.2.4");
    }

    #[test]
    fn bump_from_zero() {
        assert_eq!(bump_version("v0.0.0", Bump::Patch), "v0.0.1");
        assert_eq!(bump_version("v0.0.0", Bump::Minor), "v0.1.0");
        assert_eq!(bump_version("v0.0.0", Bump::Major), "v1.0.0");
    }

    #[test]
    fn release_items_total() {
        let items = ReleaseItems {
            bugs: vec![ReleaseItem {
                id: "X-0001".into(),
                title: "fix".into(),
            }],
            stories: vec![ReleaseItem {
                id: "X-0002".into(),
                title: "feat".into(),
            }],
            ..Default::default()
        };
        assert_eq!(items.total(), 2);
        assert!(!items.is_empty());
    }

    #[test]
    fn release_roundtrip() {
        let release = Release {
            version: "v0.4.0".into(),
            title: Some("Test release".into()),
            description: None,
            date: NaiveDate::from_ymd_opt(2026, 3, 22).unwrap(),
            previous: Some("v0.3.1".into()),
            contributors: vec![Contributor {
                id: "human:test@x.com".into(),
                events: 12,
                items: 3,
            }],
            items: ReleaseItems {
                bugs: vec![ReleaseItem {
                    id: "X-0001".into(),
                    title: "fix".into(),
                }],
                ..Default::default()
            },
        };
        let yaml = serde_yaml_ng::to_string(&release).unwrap();
        let parsed: Release = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(release, parsed);
    }
}
