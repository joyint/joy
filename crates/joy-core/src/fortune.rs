// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Random quotes and jokes for CLI output.
//!
//! Quotes are loaded from YAML files at compile time via `include_str!`.
//! Categories: tech, science, humor.

use serde::Deserialize;

const TECH_YAML: &str = include_str!("../data/fortunes/tech.yaml");
const SCIENCE_YAML: &str = include_str!("../data/fortunes/science.yaml");
const HUMOR_YAML: &str = include_str!("../data/fortunes/humor.yaml");

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Tech,
    Science,
    Humor,
    All,
}

#[derive(Debug, Clone, Deserialize)]
struct FortuneFile {
    entries: Vec<FortuneEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct FortuneEntry {
    text: String,
    #[serde(default)]
    author: Option<String>,
}

/// Return a random fortune string, or None based on probability.
///
/// - `category`: which collection to pick from (None = All)
/// - `probability`: chance of returning a fortune (0.0 to 1.0)
pub fn fortune(category: Option<&Category>, probability: f32) -> Option<String> {
    if probability <= 0.0 {
        return None;
    }
    if probability < 1.0 {
        let roll: f32 = simple_random_f32();
        if roll > probability {
            return None;
        }
    }

    let cat = category.unwrap_or(&Category::All);
    let entries = load_entries(cat);

    if entries.is_empty() {
        return None;
    }

    let idx = simple_random_usize(entries.len());
    let entry = &entries[idx];

    Some(match &entry.author {
        Some(author) => format!("{} -- {}", entry.text, author),
        None => entry.text.clone(),
    })
}

fn load_entries(category: &Category) -> Vec<FortuneEntry> {
    match category {
        Category::Tech => parse_entries(TECH_YAML),
        Category::Science => parse_entries(SCIENCE_YAML),
        Category::Humor => parse_entries(HUMOR_YAML),
        Category::All => {
            let mut all = parse_entries(TECH_YAML);
            all.extend(parse_entries(SCIENCE_YAML));
            all.extend(parse_entries(HUMOR_YAML));
            all
        }
    }
}

fn parse_entries(yaml: &str) -> Vec<FortuneEntry> {
    serde_yml::from_str::<FortuneFile>(yaml)
        .map(|f| f.entries)
        .unwrap_or_default()
}

/// Simple pseudo-random f32 in [0, 1) using system time as seed.
/// No external dependency needed for this use case.
fn simple_random_f32() -> f32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    // xorshift-like mixing
    let mut x = nanos;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    (x as f32) / (u32::MAX as f32)
}

/// Simple pseudo-random usize in [0, max) using system time as seed.
fn simple_random_usize(max: usize) -> usize {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let mut x = nanos;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    (x as usize) % max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fortune_returns_some_at_full_probability() {
        let result = fortune(Some(&Category::Tech), 1.0);
        assert!(result.is_some());
    }

    #[test]
    fn fortune_returns_none_at_zero_probability() {
        let result = fortune(Some(&Category::Tech), 0.0);
        assert!(result.is_none());
    }

    #[test]
    fn fortune_all_category_has_entries() {
        let entries = load_entries(&Category::All);
        assert!(entries.len() > 150); // 50+ per category
    }

    #[test]
    fn fortune_each_category_has_entries() {
        assert!(load_entries(&Category::Tech).len() >= 50);
        assert!(load_entries(&Category::Science).len() >= 50);
        assert!(load_entries(&Category::Humor).len() >= 50);
    }

    #[test]
    fn fortune_format_with_author() {
        let entry = FortuneEntry {
            text: "Test quote".into(),
            author: Some("Test Author".into()),
        };
        let formatted = match &entry.author {
            Some(author) => format!("{} -- {}", entry.text, author),
            None => entry.text.clone(),
        };
        assert_eq!(formatted, "Test quote -- Test Author");
    }

    #[test]
    fn fortune_format_without_author() {
        let entry = FortuneEntry {
            text: "Anonymous quote".into(),
            author: None,
        };
        let formatted = match &entry.author {
            Some(author) => format!("{} -- {}", entry.text, author),
            None => entry.text.clone(),
        };
        assert_eq!(formatted, "Anonymous quote");
    }
}
