// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! The two reference reports. Read-only over joy-core; the shapes follow
//! the desktop mockup's milestone example.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration, Utc};

use crate::nodes::{Node, Scalar};

fn tally<'a>(values: impl Iterator<Item = &'a str>) -> Vec<Vec<Scalar>> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(key, count)| vec![Scalar::from(key), Scalar::from(count)])
        .collect()
}

fn enum_str<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Milestone report: the mockup example (progress, breakdowns, effort).
pub fn milestone(root: &Path, id: &str) -> Result<Node> {
    let milestones = joy_core::milestones::load_milestones(root).context("load milestones")?;
    let milestone = milestones.iter().find(|m| m.id == id);
    let items = joy_core::items::load_items(root).context("load items")?;
    let in_milestone: Vec<_> = items
        .iter()
        .filter(|it| it.milestone.as_deref() == Some(id))
        .collect();
    if milestone.is_none() && in_milestone.is_empty() {
        bail!("unknown milestone: {id}");
    }

    let total = in_milestone.len();
    let closed = in_milestone
        .iter()
        .filter(|it| enum_str(&it.status) == "closed")
        .count();
    let progress = if total == 0 {
        0.0
    } else {
        (closed as f64 / total as f64 * 100.0).round()
    };
    let effort_total: u32 = in_milestone
        .iter()
        .filter_map(|it| it.effort.map(u32::from))
        .sum();
    let effort_done: u32 = in_milestone
        .iter()
        .filter(|it| enum_str(&it.status) == "closed")
        .filter_map(|it| it.effort.map(u32::from))
        .sum();

    let title = milestone.map(|m| m.title.clone()).unwrap_or_default();
    let label = if title.is_empty() {
        id.to_string()
    } else {
        format!("{id} {title}")
    };

    let mut children = vec![
        Node::value_with_unit("Fortschritt", progress, "%"),
        Node::value("Items", format!("{closed}/{total}")),
        Node::table(
            "Nach Status",
            &["status", "count"],
            tally(in_milestone.iter().map(|it| {
                let s = enum_str(&it.status);
                Box::leak(s.into_boxed_str()) as &str
            })),
            "bar",
        ),
        Node::table(
            "Nach Typ",
            &["type", "count"],
            tally(in_milestone.iter().map(|it| {
                let s = enum_str(&it.item_type);
                Box::leak(s.into_boxed_str()) as &str
            })),
            "pie",
        ),
    ];
    if effort_total > 0 {
        children.push(Node::value(
            "Effort",
            format!("{effort_done}/{effort_total}"),
        ));
    }
    if let Some(date) = milestone.and_then(|m| m.date) {
        children.push(Node::value("Zieldatum", date.to_string()));
    }
    Ok(Node::group(&format!("Meilenstein {label}"), children))
}

/// Parse a window like "2w" into (buckets, bucket duration, unit label).
fn parse_window(window: &str) -> Result<(i64, Duration, &'static str)> {
    let (digits, unit) = window.split_at(window.len().saturating_sub(1));
    let count: i64 = digits
        .parse()
        .with_context(|| format!("invalid window: {window} (use e.g. 2w, 48h, 30d, 3m)"))?;
    if !(1..=120).contains(&count) {
        bail!("window out of range: {window}");
    }
    Ok(match unit {
        "h" => (count, Duration::hours(1), "Stunde"),
        "d" => (count, Duration::days(1), "Tag"),
        "w" => (count, Duration::weeks(1), "Woche"),
        "m" => (count, Duration::days(30), "Monat"),
        _ => bail!("unknown window unit: {window} (h, d, w, m)"),
    })
}

/// Log timestamps are RFC3339 or naive local ("%Y-%m-%d %H:%M:%S").
fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    if let Ok(at) = raw.parse::<DateTime<Utc>>() {
        return Some(at);
    }
    chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
        .ok()
        .and_then(|naive| naive.and_local_timezone(chrono::Local).single())
        .map(|local| local.with_timezone(&Utc))
}

/// Velocity: closed items per bucket over the trailing window, from the
/// event log (ItemStatusChanged to closed).
pub fn velocity(root: &Path, window: &str) -> Result<Node> {
    let (bucket_count, bucket_len, unit_label) = parse_window(window)?;
    let entries = joy_core::event_log::read_all_events(root).context("read event log")?;
    let now = Utc::now();
    let span_start = now - bucket_len * (bucket_count as i32);

    let mut buckets = vec![0usize; bucket_count as usize];
    let mut total = 0usize;
    for entry in &entries {
        if entry.event_type != "item.status_changed" {
            continue;
        }
        let to_closed = entry
            .details
            .as_deref()
            .map(|d| d.trim_end().ends_with("-> closed"))
            .unwrap_or(false);
        if !to_closed {
            continue;
        }
        let Some(at) = parse_timestamp(&entry.timestamp) else {
            continue;
        };
        if at < span_start || at > now {
            continue;
        }
        let offset = (at - span_start).num_seconds() / bucket_len.num_seconds();
        let index = (offset.max(0) as usize).min(buckets.len() - 1);
        buckets[index] += 1;
        total += 1;
    }

    let rows: Vec<Vec<Scalar>> = buckets
        .iter()
        .enumerate()
        .map(|(i, count)| {
            let bucket_start = span_start + bucket_len * (i as i32);
            let label = match unit_label {
                "Stunde" => bucket_start.format("%H:%M").to_string(),
                _ => bucket_start.format("%d.%m.").to_string(),
            };
            vec![Scalar::Text(label), Scalar::from(*count)]
        })
        .collect();

    let per_bucket = if bucket_count > 0 {
        total as f64 / bucket_count as f64
    } else {
        0.0
    };
    Ok(Node::group(
        &format!("Velocity ({window})"),
        vec![
            Node::value("Geschlossen gesamt", total),
            Node::value_with_unit(
                &format!("Pro {unit_label}"),
                (per_bucket * 10.0).round() / 10.0,
                "Items",
            ),
            Node::table("Verlauf", &["bucket", "closed"], rows, "bar"),
        ],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_project(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("joy-bi-test-{}-{}", tag, std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("mkdir");
        joy_core::init::init(joy_core::init::InitOptions {
            root: dir.clone(),
            name: Some("BI Test".into()),
            acronym: Some("BT".into()),
            user: Some("bi@example.com".into()),
            language: None,
        })
        .expect("init");
        dir
    }

    fn add_item(root: &Path, title: &str, milestone: Option<&str>, close: bool) -> String {
        let id = joy_core::items::next_id(root, "BT", title).unwrap();
        let mut item =
            joy_core::templates::render_item(&joy_core::model::item::ItemType::Task, &id, title)
                .unwrap();
        item.milestone = milestone.map(str::to_string);
        item.effort = Some(2);
        joy_core::items::save_item(root, &item).unwrap();
        if close {
            item.status = joy_core::model::item::Status::Closed;
            joy_core::items::update_item(root, &item).unwrap();
            joy_core::event_log::log_event_as(
                root,
                joy_core::event_log::EventType::ItemStatusChanged,
                &id,
                Some("new -> closed"),
                "bi@example.com",
            );
        }
        id
    }

    #[test]
    fn milestone_report_counts_progress_and_breakdowns() {
        let dir = temp_project("ms");
        // a milestone definition is optional for the report; items carry it
        add_item(&dir, "One", Some("BT-MS-01"), true);
        add_item(&dir, "Two", Some("BT-MS-01"), false);
        add_item(&dir, "Elsewhere", None, false);

        let node = milestone(&dir, "BT-MS-01").expect("report");
        let json = serde_json::to_value(&node).unwrap();
        assert_eq!(json["kind"], "group");
        let children = json["children"].as_array().unwrap();
        assert_eq!(children[0]["label"], "Fortschritt");
        assert_eq!(children[0]["value"], 50.0);
        assert_eq!(children[1]["value"], "1/2");
        assert_eq!(children[2]["view"], "bar");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn milestone_report_refuses_unknown_ids() {
        let dir = temp_project("unknown");
        assert!(milestone(&dir, "BT-MS-99").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn velocity_buckets_closed_events() {
        let dir = temp_project("velocity");
        add_item(&dir, "Closed now", None, true);
        add_item(&dir, "Open", None, false);

        let node = velocity(&dir, "2d").expect("report");
        let json = serde_json::to_value(&node).unwrap();
        assert_eq!(json["children"][0]["value"], 1.0);
        let rows = json["children"][2]["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn window_units_parse_and_refuse_garbage() {
        assert!(parse_window("2w").is_ok());
        assert!(parse_window("48h").is_ok());
        assert!(parse_window("30d").is_ok());
        assert!(parse_window("3m").is_ok());
        assert!(parse_window("2y").is_err());
        assert!(parse_window("w").is_err());
    }
}
