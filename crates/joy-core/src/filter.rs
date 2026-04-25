// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Declarative filter for item listing views.
//!
//! `FilterSpec` is a CLI-free description of "which items should we show".
//! Listing views (ls, board, roadmap) build a spec from their CLI args
//! and call [`apply`] to materialise the matching items. Identity-aware
//! resolution (e.g. `--mine` -> a concrete member ID) lives in the
//! caller; this module only consumes already-resolved values.

use std::collections::HashSet;

use crate::model::item::{Item, ItemType, Priority, Status};

/// Declarative item filter. Empty / `None` fields mean "no filter on this
/// dimension". The `all` flag reproduces the historical default that
/// closed and deferred items are hidden unless either `all` is set or a
/// `status` filter explicitly requests them.
#[derive(Debug, Clone, Default)]
pub struct FilterSpec {
    pub parent: Option<String>,
    pub item_type: Option<ItemType>,
    pub status: Option<Status>,
    pub priority: Option<Priority>,
    pub milestone: Option<String>,
    pub tag: Option<String>,
    pub version: Option<String>,
    /// Match items where at least one assignee's member ID is in this list.
    /// Empty means no member filter.
    pub members: Vec<String>,
    pub blocked: bool,
    /// Include closed and deferred items.
    pub all: bool,
}

/// Apply the spec against `all_items`, returning matching items in input order.
pub fn apply<'a>(all_items: &'a [Item], spec: &FilterSpec) -> Vec<&'a Item> {
    all_items
        .iter()
        .filter(|item| matches_spec(item, spec, all_items))
        .collect()
}

fn matches_spec(item: &Item, spec: &FilterSpec, all_items: &[Item]) -> bool {
    if !spec.all
        && spec.status.is_none()
        && matches!(item.status, Status::Closed | Status::Deferred)
    {
        return false;
    }

    if let Some(ref parent_id) = spec.parent {
        if item.id != *parent_id && !is_descendant(item, parent_id, all_items) {
            return false;
        }
    }

    if let Some(ref t) = spec.item_type {
        if &item.item_type != t {
            return false;
        }
    }

    if let Some(ref s) = spec.status {
        if &item.status != s {
            return false;
        }
    }

    if let Some(ref p) = spec.priority {
        if &item.priority != p {
            return false;
        }
    }

    if let Some(ref ms) = spec.milestone {
        if effective_milestone(item, all_items) != Some(ms.as_str()) {
            return false;
        }
    }

    if let Some(ref tag) = spec.tag {
        if !item.tags.iter().any(|t| t == tag) {
            return false;
        }
    }

    if let Some(ref version) = spec.version {
        if item.version.as_deref() != Some(version.as_str()) {
            return false;
        }
    }

    if !spec.members.is_empty()
        && !item
            .assignees
            .iter()
            .any(|a| spec.members.iter().any(|m| m == &a.member))
    {
        return false;
    }

    if spec.blocked && !item.is_blocked_by(all_items) {
        return false;
    }

    true
}

/// Resolve the milestone an item effectively belongs to: its own, or
/// inherited from its closest ancestor that has one. Returns `None` if
/// no ancestor in the chain declares a milestone.
pub fn effective_milestone<'a>(item: &'a Item, all_items: &'a [Item]) -> Option<&'a str> {
    if let Some(ref ms) = item.milestone {
        return Some(ms.as_str());
    }
    let mut visited = HashSet::new();
    let mut current_parent = item.parent.as_deref();
    while let Some(pid) = current_parent {
        if !visited.insert(pid) {
            break;
        }
        if let Some(parent) = all_items.iter().find(|i| i.id == pid) {
            if let Some(ref ms) = parent.milestone {
                return Some(ms.as_str());
            }
            current_parent = parent.parent.as_deref();
        } else {
            break;
        }
    }
    None
}

/// True if `item` is a descendant of `ancestor_id` in the parent chain.
pub fn is_descendant(item: &Item, ancestor_id: &str, all_items: &[Item]) -> bool {
    let mut visited = HashSet::new();
    let mut current = item.parent.as_deref();
    while let Some(pid) = current {
        if !visited.insert(pid) {
            break;
        }
        if pid == ancestor_id {
            return true;
        }
        current = all_items
            .iter()
            .find(|i| i.id == pid)
            .and_then(|i| i.parent.as_deref());
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::item::Assignee;
    use chrono::Utc;

    fn make(id: &str) -> Item {
        let now = Utc::now();
        Item {
            id: id.to_string(),
            title: id.to_string(),
            item_type: ItemType::Task,
            status: Status::New,
            priority: Priority::Medium,
            parent: None,
            assignees: vec![],
            deps: vec![],
            milestone: None,
            tags: vec![],
            capabilities: vec![],
            mode: None,
            effort: None,
            version: None,
            created_by: None,
            created: now,
            updated: now,
            description: None,
            comments: vec![],
        }
    }

    fn ids(items: &[&Item]) -> Vec<String> {
        items.iter().map(|i| i.id.clone()).collect()
    }

    #[test]
    fn default_spec_hides_closed_and_deferred() {
        let mut a = make("A");
        a.status = Status::Closed;
        let mut b = make("B");
        b.status = Status::Deferred;
        let c = make("C");
        let items = vec![a, b, c];
        let result = apply(&items, &FilterSpec::default());
        assert_eq!(ids(&result), vec!["C"]);
    }

    #[test]
    fn all_flag_includes_closed_and_deferred() {
        let mut a = make("A");
        a.status = Status::Closed;
        let mut b = make("B");
        b.status = Status::Deferred;
        let c = make("C");
        let items = vec![a, b, c];
        let spec = FilterSpec {
            all: true,
            ..Default::default()
        };
        let result = apply(&items, &spec);
        assert_eq!(ids(&result), vec!["A", "B", "C"]);
    }

    #[test]
    fn status_filter_keeps_closed_when_explicitly_requested() {
        let mut a = make("A");
        a.status = Status::Closed;
        let b = make("B");
        let items = vec![a, b];
        let spec = FilterSpec {
            status: Some(Status::Closed),
            ..Default::default()
        };
        let result = apply(&items, &spec);
        assert_eq!(ids(&result), vec!["A"]);
    }

    #[test]
    fn type_filter_matches_exactly() {
        let a = make("A");
        let mut b = make("B");
        b.item_type = ItemType::Bug;
        let items = vec![a, b];
        let spec = FilterSpec {
            item_type: Some(ItemType::Bug),
            ..Default::default()
        };
        assert_eq!(ids(&apply(&items, &spec)), vec!["B"]);
    }

    #[test]
    fn priority_filter_matches_exactly() {
        let a = make("A");
        let mut b = make("B");
        b.priority = Priority::Critical;
        let items = vec![a, b];
        let spec = FilterSpec {
            priority: Some(Priority::Critical),
            ..Default::default()
        };
        assert_eq!(ids(&apply(&items, &spec)), vec!["B"]);
    }

    #[test]
    fn milestone_filter_matches_own_milestone() {
        let mut a = make("A");
        a.milestone = Some("MS1".to_string());
        let b = make("B");
        let items = vec![a, b];
        let spec = FilterSpec {
            milestone: Some("MS1".to_string()),
            ..Default::default()
        };
        assert_eq!(ids(&apply(&items, &spec)), vec!["A"]);
    }

    #[test]
    fn milestone_filter_matches_inherited_from_parent() {
        let mut parent = make("P");
        parent.milestone = Some("MS1".to_string());
        let mut child = make("C");
        child.parent = Some("P".to_string());
        let items = vec![parent, child];
        let spec = FilterSpec {
            milestone: Some("MS1".to_string()),
            ..Default::default()
        };
        assert_eq!(ids(&apply(&items, &spec)), vec!["P", "C"]);
    }

    #[test]
    fn tag_filter_matches_when_tag_present() {
        let mut a = make("A");
        a.tags = vec!["frontend".to_string()];
        let b = make("B");
        let items = vec![a, b];
        let spec = FilterSpec {
            tag: Some("frontend".to_string()),
            ..Default::default()
        };
        assert_eq!(ids(&apply(&items, &spec)), vec!["A"]);
    }

    #[test]
    fn version_filter_matches_exactly() {
        let mut a = make("A");
        a.version = Some("v0.5.0".to_string());
        let b = make("B");
        let items = vec![a, b];
        let spec = FilterSpec {
            version: Some("v0.5.0".to_string()),
            ..Default::default()
        };
        assert_eq!(ids(&apply(&items, &spec)), vec!["A"]);
    }

    #[test]
    fn member_filter_matches_any_listed() {
        let mut a = make("A");
        a.assignees = vec![Assignee {
            member: "alice@example.com".to_string(),
            capabilities: vec![],
        }];
        let mut b = make("B");
        b.assignees = vec![Assignee {
            member: "bob@example.com".to_string(),
            capabilities: vec![],
        }];
        let c = make("C");
        let items = vec![a, b, c];
        let spec = FilterSpec {
            members: vec!["alice@example.com".to_string(), "bob@example.com".to_string()],
            ..Default::default()
        };
        assert_eq!(ids(&apply(&items, &spec)), vec!["A", "B"]);
    }

    #[test]
    fn empty_member_list_is_no_filter() {
        let a = make("A");
        let mut b = make("B");
        b.assignees = vec![Assignee {
            member: "alice@example.com".to_string(),
            capabilities: vec![],
        }];
        let items = vec![a, b];
        assert_eq!(ids(&apply(&items, &FilterSpec::default())), vec!["A", "B"]);
    }

    #[test]
    fn parent_filter_includes_self_and_descendants() {
        let parent = make("P");
        let mut child = make("C");
        child.parent = Some("P".to_string());
        let mut grandchild = make("G");
        grandchild.parent = Some("C".to_string());
        let other = make("O");
        let items = vec![parent, child, grandchild, other];
        let spec = FilterSpec {
            parent: Some("P".to_string()),
            ..Default::default()
        };
        assert_eq!(ids(&apply(&items, &spec)), vec!["P", "C", "G"]);
    }

    #[test]
    fn is_descendant_handles_cycles() {
        let mut a = make("A");
        a.parent = Some("B".to_string());
        let mut b = make("B");
        b.parent = Some("A".to_string());
        let items = vec![a.clone(), b];
        assert!(!is_descendant(&a, "X", &items));
    }
}
