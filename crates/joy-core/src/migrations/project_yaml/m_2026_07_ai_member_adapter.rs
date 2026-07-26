// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Fill the missing ACP adapter pin on AI members (JI-0164).
//!
//! `adapter` was added to the project.yaml member so a host can route a
//! turn without a per-member agent file. Members registered before it —
//! `joy ai init` never wrote the field — carry none, which left every
//! surface that reads the pin with nothing: the app rendered an empty
//! adapter badge next to the member name, and a host had to guess.
//!
//! The canonical member id determines the adapter unambiguously
//! (`ai:<tool>@joy`), so the value is derivable and no one has to retype
//! it. Members that already carry a pin are left exactly as they are.
//!
//! The tool -> adapter table is inlined ON PURPOSE. A migration is a
//! historical record: it must reproduce the mapping as it stood when the
//! data was written, so a later rename of the adapter ids (its own
//! migration) cannot retroactively change what this one produces.

use serde_yaml_ng::Value;

/// The adapter a tool id ran on when the pin was introduced (2026-07).
/// Frozen by design; see the module note.
fn adapter_for_tool(tool: &str) -> Option<&'static str> {
    match tool {
        "claude" => Some("claude-code"),
        "qwen" => Some("qwen-code"),
        "vibe" => Some("mistral-vibe"),
        "copilot" => Some("copilot"),
        _ => None,
    }
}

/// The tool id inside a canonical AI member key (`ai:vibe@joy` -> `vibe`).
fn tool_of(member_key: &str) -> Option<&str> {
    member_key.strip_prefix("ai:")?.split('@').next()
}

pub fn migrate(mut value: Value) -> (Value, bool) {
    let mut changed = false;
    let Some(members) = value.get_mut("members").and_then(Value::as_mapping_mut) else {
        return (value, changed);
    };
    for (key, member) in members.iter_mut() {
        let Some(key) = key.as_str() else { continue };
        let Some(member) = member.as_mapping_mut() else {
            continue;
        };
        // Only a canonical AI member has a derivable adapter, and only an
        // ABSENT (or blank) pin is filled — never overwrite a recorded one.
        let missing = match member.get("adapter") {
            None => true,
            Some(Value::Null) => true,
            Some(Value::String(s)) => s.trim().is_empty(),
            Some(_) => false,
        };
        if !missing {
            continue;
        }
        let Some(adapter) = tool_of(key).and_then(adapter_for_tool) else {
            continue;
        };
        member.insert(
            Value::String("adapter".into()),
            Value::String(adapter.into()),
        );
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
    fn fills_the_pin_for_ai_members_that_lack_it() {
        let (out, changed) = migrate(yaml(
            r#"
members:
  horst@example.com:
    capabilities: all
  ai:claude@joy:
    capabilities: all
  ai:vibe@joy:
    capabilities: all
"#,
        ));
        assert!(changed);
        let members = out.get("members").unwrap();
        assert_eq!(
            members
                .get("ai:claude@joy")
                .unwrap()
                .get("adapter")
                .unwrap(),
            &Value::String("claude-code".into())
        );
        assert_eq!(
            members.get("ai:vibe@joy").unwrap().get("adapter").unwrap(),
            &Value::String("mistral-vibe".into())
        );
        // a human member is untouched
        assert!(members
            .get("horst@example.com")
            .unwrap()
            .get("adapter")
            .is_none());
    }

    #[test]
    fn never_overwrites_a_recorded_pin_and_is_idempotent() {
        let source = r#"
members:
  ai:claude@joy:
    adapter: something-custom
  ai:qwen@joy:
    adapter: ""
"#;
        let (out, changed) = migrate(yaml(source));
        assert!(changed, "the blank pin is filled");
        let members = out.get("members").unwrap();
        assert_eq!(
            members
                .get("ai:claude@joy")
                .unwrap()
                .get("adapter")
                .unwrap(),
            &Value::String("something-custom".into()),
            "a recorded pin is authoritative"
        );
        assert_eq!(
            members.get("ai:qwen@joy").unwrap().get("adapter").unwrap(),
            &Value::String("qwen-code".into())
        );
        // running it again changes nothing
        let (_again, changed_again) = migrate(out);
        assert!(!changed_again, "idempotent");
    }

    #[test]
    fn leaves_unknown_tools_and_malformed_shapes_alone() {
        let (out, changed) = migrate(yaml(
            r#"
members:
  ai:mock@joy:
    capabilities: all
  not-a-member: 7
"#,
        ));
        assert!(!changed);
        assert!(out
            .get("members")
            .unwrap()
            .get("ai:mock@joy")
            .unwrap()
            .get("adapter")
            .is_none());
    }
}
