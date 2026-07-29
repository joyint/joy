// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Rename the recorded adapter pins to the tool names (JOY-0231-74).
//!
//! The first generation of adapter ids carried the provider in the name
//! (`mistral-vibe`, `claude-code`, `qwen-code`). Since JI-017A-85 the
//! adapter id IS the tool name (`vibe`, `claude`, `qwen`) — one word for
//! the tool everywhere: the pin, the registry row, the container's
//! JOY_ADAPTER, the image routing. This migration rewrites pins recorded
//! in the old spelling; absent pins were already backfilled with the old
//! spelling by `m_2026_07_ai_member_adapter` (frozen by design) and are
//! rewritten here in the same pass.
//!
//! The legacy -> tool table is inlined ON PURPOSE: a migration is a
//! historical record of the spellings as they stood in 2026-07, even if
//! the live registry (joy-ai) drops its legacy list one day.

use serde_yaml_ng::Value;

/// The tool name behind a first-generation adapter id. Frozen by design;
/// see the module note. Anything else — already-migrated pins, `mock`,
/// custom values — is left exactly as recorded.
fn tool_name_for(adapter: &str) -> Option<&'static str> {
    match adapter {
        "claude-code" => Some("claude"),
        "qwen-code" => Some("qwen"),
        "mistral-vibe" => Some("vibe"),
        _ => None,
    }
}

pub fn migrate(mut value: Value) -> (Value, bool) {
    let mut changed = false;
    let Some(members) = value.get_mut("members").and_then(Value::as_mapping_mut) else {
        return (value, changed);
    };
    for (_, member) in members.iter_mut() {
        let Some(member) = member.as_mapping_mut() else {
            continue;
        };
        let Some(Value::String(recorded)) = member.get("adapter") else {
            continue;
        };
        let Some(tool) = tool_name_for(recorded) else {
            continue;
        };
        member.insert(Value::String("adapter".into()), Value::String(tool.into()));
        changed = true;
    }
    (value, changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(text: &str) -> Value {
        serde_yaml_ng::from_str(text).unwrap()
    }

    #[test]
    fn rewrites_every_first_generation_pin_to_its_tool_name() {
        let (out, changed) = migrate(yaml(
            r#"
members:
  ai:claude@joy:
    adapter: claude-code
  ai:qwen@joy:
    adapter: qwen-code
  ai:vibe@joy:
    adapter: mistral-vibe
"#,
        ));
        assert!(changed);
        let members = out.get("members").unwrap();
        for (member, tool) in [
            ("ai:claude@joy", "claude"),
            ("ai:qwen@joy", "qwen"),
            ("ai:vibe@joy", "vibe"),
        ] {
            assert_eq!(
                members.get(member).unwrap().get("adapter").unwrap(),
                &Value::String(tool.into())
            );
        }
    }

    #[test]
    fn leaves_migrated_custom_and_absent_pins_alone_and_is_idempotent() {
        let source = r#"
members:
  ai:vibe@joy:
    adapter: vibe
  ai:claude@joy:
    adapter: something-custom
  ai:mock@joy:
    adapter: mock
  horst@example.com:
    capabilities: all
"#;
        let (out, changed) = migrate(yaml(source));
        assert!(!changed, "nothing in the old spelling: nothing to do");
        let (_again, changed_again) = migrate(out);
        assert!(!changed_again, "idempotent");
    }

    #[test]
    fn composes_with_the_frozen_backfill() {
        // The backfill (frozen) writes the OLD spelling for members that
        // never carried a pin; this migration renames it in the same
        // apply() pass, so a fresh read serves tool names either way.
        let (backfilled, _) = super::super::m_2026_07_ai_member_adapter::migrate(yaml(
            r#"
members:
  ai:vibe@joy:
    capabilities: all
"#,
        ));
        let (out, changed) = migrate(backfilled);
        assert!(changed);
        assert_eq!(
            out.get("members")
                .unwrap()
                .get("ai:vibe@joy")
                .unwrap()
                .get("adapter")
                .unwrap(),
            &Value::String("vibe".into())
        );
    }
}
