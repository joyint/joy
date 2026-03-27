// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use chrono::Utc;
use minijinja::Environment;

use crate::error::JoyError;
use crate::model::item::{Item, ItemType};

// Embedded item templates
const BASE_TEMPLATE: &str = include_str!("../../../data/items/_base.yaml");
const EPIC_TEMPLATE: &str = include_str!("../../../data/items/epic.yaml");
const STORY_TEMPLATE: &str = include_str!("../../../data/items/story.yaml");
const TASK_TEMPLATE: &str = include_str!("../../../data/items/task.yaml");
const BUG_TEMPLATE: &str = include_str!("../../../data/items/bug.yaml");
const REWORK_TEMPLATE: &str = include_str!("../../../data/items/rework.yaml");
const DECISION_TEMPLATE: &str = include_str!("../../../data/items/decision.yaml");
const IDEA_TEMPLATE: &str = include_str!("../../../data/items/idea.yaml");

fn template_for_type(item_type: &ItemType) -> (&'static str, &'static str) {
    match item_type {
        ItemType::Epic => ("epic.yaml", EPIC_TEMPLATE),
        ItemType::Story => ("story.yaml", STORY_TEMPLATE),
        ItemType::Task => ("task.yaml", TASK_TEMPLATE),
        ItemType::Bug => ("bug.yaml", BUG_TEMPLATE),
        ItemType::Rework => ("rework.yaml", REWORK_TEMPLATE),
        ItemType::Decision => ("decision.yaml", DECISION_TEMPLATE),
        ItemType::Idea => ("idea.yaml", IDEA_TEMPLATE),
    }
}

/// Render an item template for the given type, filling in id and title.
/// Returns a deserialized Item ready for further modification.
pub fn render_item(item_type: &ItemType, id: &str, title: &str) -> Result<Item, JoyError> {
    let mut env = Environment::new();
    env.add_template("_base.yaml", BASE_TEMPLATE)
        .map_err(|e| JoyError::Template(e.to_string()))?;

    let (name, source) = template_for_type(item_type);
    env.add_template(name, source)
        .map_err(|e| JoyError::Template(e.to_string()))?;

    let now = Utc::now().to_rfc3339();
    let tmpl = env
        .get_template(name)
        .map_err(|e| JoyError::Template(e.to_string()))?;

    let escaped_title = title.replace('\\', "\\\\").replace('"', "\\\"");
    let yaml = tmpl
        .render(minijinja::context! {
            id => id,
            title => escaped_title,
            now => &now,
        })
        .map_err(|e| JoyError::Template(e.to_string()))?;

    let item: Item =
        serde_yaml_ng::from_str(&yaml).map_err(|e| JoyError::Template(e.to_string()))?;

    Ok(item)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_story_template() {
        let item = render_item(&ItemType::Story, "JOY-0001", "Login page").unwrap();
        assert_eq!(item.id, "JOY-0001");
        assert_eq!(item.title, "Login page");
        assert_eq!(item.item_type, ItemType::Story);
        assert_eq!(item.capabilities.len(), 3); // plan, implement, review
    }

    #[test]
    fn render_idea_template() {
        let item = render_item(&ItemType::Idea, "JOY-0002", "Wild thought").unwrap();
        assert_eq!(item.item_type, ItemType::Idea);
        assert!(item.capabilities.is_empty());
    }

    #[test]
    fn render_title_with_colon() {
        let item = render_item(&ItemType::Bug, "JOY-005A", "Fix: crash on startup").unwrap();
        assert_eq!(item.title, "Fix: crash on startup");
    }

    #[test]
    fn render_title_with_special_yaml_chars() {
        let item = render_item(
            &ItemType::Task,
            "JOY-0099",
            r#"Handle "quotes" & {braces} [brackets]"#,
        )
        .unwrap();
        assert_eq!(item.title, r#"Handle "quotes" & {braces} [brackets]"#);
    }

    #[test]
    fn render_all_types() {
        let types = [
            ItemType::Epic,
            ItemType::Story,
            ItemType::Task,
            ItemType::Bug,
            ItemType::Rework,
            ItemType::Decision,
            ItemType::Idea,
        ];
        for t in &types {
            let item = render_item(t, "TEST-0001", "Test item").unwrap();
            assert_eq!(item.item_type, *t);
        }
    }
}
