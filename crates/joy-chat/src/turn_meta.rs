// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The execution record of ONE AI chat turn: which level it ran under,
//! which model answered, what it cost and how many tokens it burned.
//!
//! This lives here, in the shared chat crate, because BOTH hosts write it
//! and one shared reader renders it. The platform folds it in after a
//! server-side turn, the desktop after a local ACP turn, and the app's
//! chat channel parses the identical keys either way. When the two hosts
//! each kept their own copy of this logic, the desktop silently wrote a
//! poorer record than the web: no level beside the member's name, no
//! model, no cost, no tokens in the turn-info popover (operator
//! 2026-07-26). One producer, one shape, no host-specific truth.

use std::path::Path;

use joy_core::model::config::InteractionLevel;

/// What a finished turn reports about itself. Every field is optional:
/// a host contributes what it can observe, and the rest stays absent
/// rather than being invented (a local project has no month budget, so
/// the budget row simply does not appear there).
#[derive(Debug, Default, Clone)]
pub struct TurnMeta<'a> {
    /// The model that answered, as the agent reported it.
    pub model: Option<&'a str>,
    /// This call's spend, in cents.
    pub cost_cents: Option<u64>,
    /// Tokens this turn used, from the ACP usage update.
    pub tokens: Option<u64>,
    /// The interaction level the turn actually ran under (JI-0166-D8 §5).
    pub interaction_level: Option<&'a str>,
    /// The member's spend after this turn (platform only).
    pub spent_cents: Option<u64>,
    /// The effective monthly cap it counts against (platform only).
    pub cap_cents: Option<u64>,
}

impl TurnMeta<'_> {
    /// Whether there is anything worth folding in. `spent_cents` does not
    /// count: a running total is a companion to a record, never a reason
    /// to start one — a turn that reports nothing about itself should not
    /// grow a details blob just to say "spent nothing".
    fn is_empty(&self) -> bool {
        self.model.is_none_or(str::is_empty)
            && self.cost_cents.is_none()
            && self.tokens.is_none()
            && self.interaction_level.is_none_or(str::is_empty)
            && self.cap_cents.is_none()
    }
}

/// Fold the turn's execution record into the activity details JSON
/// (JI-014A/JI-0162): the message's info popover shows the level, the
/// model, this call's cost and tokens, and — where a budget exists — the
/// spend against the cap.
///
/// It rides in `details`, which is already persisted and sealed per
/// message, so neither the wire nor the joy-core schema has to change.
/// Returns the input unchanged when there is nothing to add.
pub fn augment_details(details: Option<String>, meta: &TurnMeta<'_>) -> Option<String> {
    // Nothing to fold in leaves the block byte-identical: re-serializing an
    // untouched activity block would only churn its key order.
    if meta.is_empty() {
        return details;
    }
    let mut obj = details
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(s).ok())
        .unwrap_or_else(|| {
            let mut m = serde_json::Map::new();
            m.insert("v".into(), serde_json::json!(1));
            m
        });
    if let Some(model) = meta.model.filter(|m| !m.is_empty()) {
        obj.insert("model".into(), serde_json::json!(model));
    }
    if let Some(cost) = meta.cost_cents {
        obj.insert("cost_cents".into(), serde_json::json!(cost));
    }
    if let Some(tokens) = meta.tokens {
        obj.insert("tokens".into(), serde_json::json!(tokens));
    }
    // The level this turn actually ran under: the execution record for
    // the turn header, so an effective level is verifiable after the
    // fact (E2E, activity UI) instead of only being current policy.
    if let Some(level) = meta.interaction_level.filter(|l| !l.is_empty()) {
        obj.insert("interactionLevel".into(), serde_json::json!(level));
    }
    if let Some(spent) = meta.spent_cents {
        obj.insert("spent_cents".into(), serde_json::json!(spent));
    }
    if let Some(cap) = meta.cap_cents {
        obj.insert("cap_cents".into(), serde_json::json!(cap));
    }
    serde_json::to_string(&obj).ok().or(details)
}

/// Resolve the level `delegator`'s turns of `agent` run under in this
/// chat (JI-0166-D8 §5): the per-chat, per-delegator override when
/// stored, else the AGENT MEMBER's default level from project.yaml
/// (member entry, else the project defaults global).
///
/// ONE resolution for every caller — the platform's read path and turn
/// loop, and the desktop's local turn. They must never disagree. Chat
/// turns carry no capability context, so the per-capability
/// max-interaction-level floors do not apply here (they clamp
/// capability-scoped work).
pub fn resolve_effective_level(
    dir: &Path,
    chat: &crate::model::chat::Chat,
    agent: &str,
    delegator: &str,
    personal: Option<InteractionLevel>,
) -> InteractionLevel {
    // ADR-025 order (JP-0099-66): chat override > the caller's personal
    // overall > member default > project default.
    let default_level = personal.unwrap_or_else(|| {
        joy_core::store::load_project(dir)
            .ok()
            .and_then(|p| p.member_by_key(agent).and_then(|m| m.interaction_level))
            .unwrap_or_else(|| joy_core::store::load_interaction_level_defaults(dir).default)
    });
    crate::model::effective_level(
        None,
        chat.interaction_level_override(agent, delegator),
        default_level,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_record_leaves_the_activity_block_alone() {
        assert_eq!(augment_details(None, &TurnMeta::default()), None);
        let block = Some(r#"{"v":1,"tools":[]}"#.to_string());
        assert_eq!(augment_details(block.clone(), &TurnMeta::default()), block);
    }

    #[test]
    fn a_local_turn_records_level_model_cost_and_tokens_without_a_budget() {
        // The desktop case: no month budget exists, so the budget keys
        // stay absent instead of being faked as zero.
        let raw = augment_details(
            None,
            &TurnMeta {
                model: Some("mistral-large"),
                cost_cents: Some(12),
                tokens: Some(3400),
                interaction_level: Some("autonomous"),
                ..Default::default()
            },
        )
        .expect("a record");
        let v: serde_json::Value = serde_json::from_str(&raw).expect("json");
        assert_eq!(v["v"], 1);
        assert_eq!(v["model"], "mistral-large");
        assert_eq!(v["cost_cents"], 12);
        assert_eq!(v["tokens"], 3400);
        assert_eq!(v["interactionLevel"], "autonomous");
        assert!(v.get("spent_cents").is_none());
        assert!(v.get("cap_cents").is_none());
    }

    #[test]
    fn the_record_joins_an_existing_activity_block_instead_of_replacing_it() {
        let block = Some(r#"{"v":1,"thoughts":"hm","tools":[{"title":"Read"}]}"#.to_string());
        let raw = augment_details(
            block,
            &TurnMeta {
                cost_cents: Some(5),
                spent_cents: Some(500),
                cap_cents: Some(2000),
                ..Default::default()
            },
        )
        .expect("a record");
        let v: serde_json::Value = serde_json::from_str(&raw).expect("json");
        assert_eq!(v["thoughts"], "hm");
        assert_eq!(v["tools"][0]["title"], "Read");
        assert_eq!(v["cost_cents"], 5);
        assert_eq!(v["spent_cents"], 500);
        assert_eq!(v["cap_cents"], 2000);
    }

    #[test]
    fn empty_strings_are_not_a_value() {
        let meta = TurnMeta {
            model: Some(""),
            interaction_level: Some(""),
            ..Default::default()
        };
        assert_eq!(augment_details(None, &meta), None);
    }
}
