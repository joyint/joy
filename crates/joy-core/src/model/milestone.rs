// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Milestone {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<NaiveDate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Milestone {
    pub fn new(id: String, title: String) -> Self {
        Self {
            id,
            title,
            date: None,
            description: None,
        }
    }
}

/// Generate a slug from a milestone title.
pub fn milestone_filename(id: &str, title: &str) -> String {
    use crate::model::item::slugify;
    format!("{}-{}.yaml", id, slugify(title))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn milestone_roundtrip() {
        let ms = Milestone {
            id: "MS-01".into(),
            title: "Beta Release".into(),
            date: Some(NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()),
            description: Some("First public beta.".into()),
        };

        let yaml = serde_yaml_ng::to_string(&ms).unwrap();
        let parsed: Milestone = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(ms, parsed);
    }

    #[test]
    fn milestone_filename_basic() {
        assert_eq!(
            milestone_filename("MS-01", "Beta Release"),
            "MS-01-beta-release.yaml"
        );
    }
}
