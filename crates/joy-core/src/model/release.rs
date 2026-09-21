// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// A contributor to a release with item count.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Contributor {
    /// The contributing member. Resolves to name/e-mail on display and in
    /// `--json`; persisted and rendered into published notes as the raw id via
    /// [`id`](crate::member_ref::MemberRef::id) so notes stay anonymous (ADR-042).
    pub id: crate::member_ref::MemberRef,
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

/// Parse a version string into (major, minor, patch, Option<prerelease>).
pub fn parse_version_parts(v: &str) -> (u64, u64, u64, Option<String>) {
    let trimmed = v.strip_prefix('v').unwrap_or(v);
    if let Ok(sem) = semver::Version::parse(trimmed) {
        let pre = if sem.pre.is_empty() {
            None
        } else {
            Some(sem.pre.to_string())
        };
        return (sem.major, sem.minor, sem.patch, pre);
    }
    let (core, pre) = match trimmed.split_once(['-', '+']) {
        Some((c, p)) => (c, Some(p.to_string())),
        None => (trimmed, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    let major = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let patch = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor, patch, pre)
}

/// Compute the next semver version from a current version string.
pub fn bump_version(current: &str, bump: Bump) -> String {
    bump_version_with_pre(current, bump, None)
}

/// Compute the next semver version with an optional prerelease label (e.g. "alpha", "beta", "rc1").
pub fn bump_version_with_pre(current: &str, bump: Bump, prerelease: Option<&str>) -> String {
    let (major, minor, patch, cur_pre) = parse_version_parts(current);
    let target_pre = prerelease
        .filter(|p| !p.trim().is_empty())
        .map(|p| p.trim().strip_prefix('-').unwrap_or(p.trim()));

    match target_pre {
        None => {
            // Target is a stable version.
            if cur_pre.is_some() {
                // When current is already a prerelease (e.g. 0.30.0-beta or 1.2.3-rc1),
                // graduating keyword bumps strip the prerelease without skipping a version:
                match bump {
                    Bump::Patch => format!("v{major}.{minor}.{patch}"),
                    Bump::Minor => {
                        if patch == 0 {
                            format!("v{major}.{minor}.0")
                        } else {
                            format!("v{major}.{}.0", minor + 1)
                        }
                    }
                    Bump::Major => {
                        if minor == 0 && patch == 0 {
                            format!("v{major}.0.0")
                        } else {
                            format!("v{}.0.0", major + 1)
                        }
                    }
                }
            } else {
                match bump {
                    Bump::Major => format!("v{}.0.0", major + 1),
                    Bump::Minor => format!("v{}.{}.0", major, minor + 1),
                    Bump::Patch => format!("v{}.{}.{}", major, minor, patch + 1),
                }
            }
        }
        Some(label) => {
            let (next_maj, next_min, next_pat) = if cur_pre.is_some() {
                match bump {
                    Bump::Patch => (major, minor, patch),
                    Bump::Minor => {
                        if patch == 0 {
                            (major, minor, 0)
                        } else {
                            (major, minor + 1, 0)
                        }
                    }
                    Bump::Major => {
                        if minor == 0 && patch == 0 {
                            (major, 0, 0)
                        } else {
                            (major + 1, 0, 0)
                        }
                    }
                }
            } else {
                match bump {
                    Bump::Major => (major + 1, 0, 0),
                    Bump::Minor => (major, minor + 1, 0),
                    Bump::Patch => (major, minor, patch + 1),
                }
            };

            let next_pre = match cur_pre {
                Some(ref cp) if cp == label => {
                    format!("{label}.1")
                }
                Some(ref cp) if cp.starts_with(&format!("{label}.")) => {
                    let suffix = &cp[label.len() + 1..];
                    if let Ok(n) = suffix.parse::<u64>() {
                        format!("{label}.{}", n + 1)
                    } else {
                        label.to_string()
                    }
                }
                _ => label.to_string(),
            };

            format!("v{next_maj}.{next_min}.{next_pat}-{next_pre}")
        }
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
    fn bump_prerelease_bases_graduate_to_stable() {
        // JOY-02AE-84: keyword bump from prerelease base does not skip or go backwards
        assert_eq!(bump_version("v0.30.0-beta", Bump::Patch), "v0.30.0");
        assert_eq!(bump_version("0.30.0-beta", Bump::Patch), "v0.30.0");
        assert_eq!(bump_version("v0.30.0-beta.1", Bump::Patch), "v0.30.0");
        assert_eq!(bump_version("v1.2.3-rc1", Bump::Patch), "v1.2.3");
        assert_eq!(bump_version("v0.30.0-beta", Bump::Minor), "v0.30.0");
    }

    #[test]
    fn bump_with_prerelease_labels() {
        assert_eq!(
            bump_version_with_pre("0.4.0", Bump::Minor, Some("alpha")),
            "v0.5.0-alpha"
        );
        assert_eq!(
            bump_version_with_pre("v0.5.0-alpha", Bump::Minor, Some("beta")),
            "v0.5.0-beta"
        );
        assert_eq!(
            bump_version_with_pre("v0.5.0-beta", Bump::Minor, None),
            "v0.5.0"
        );
        assert_eq!(
            bump_version_with_pre("v0.5.0", Bump::Patch, Some("beta")),
            "v0.5.1-beta"
        );
        assert_eq!(
            bump_version_with_pre("v0.5.0-alpha", Bump::Minor, Some("alpha")),
            "v0.5.0-alpha.1"
        );
        assert_eq!(
            bump_version_with_pre("v0.5.0-alpha.1", Bump::Minor, Some("alpha")),
            "v0.5.0-alpha.2"
        );
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
