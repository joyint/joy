// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! THE adapter registry (JI-017A-85, decided 2026-07-29): one row per AI
//! tool, holding every fact both hosts need — and nothing else. The row
//! is data, not behavior; the ACP runtime around it (lanes, sessions,
//! budgets, container health) lives with the hosts.
//!
//! The adapter id IS the tool name (JOY-0231-74): `vibe`, `claude`,
//! `qwen`. The provider-flavored ids of the first generation
//! (`mistral-vibe`, `claude-code`, `qwen-code`) stay readable as
//! `legacy_ids` while project.yaml pins migrate through joy's silent
//! update path.
//!
//! The entrypoint is ONE argv prefix, valid verbatim on the desktop PATH
//! and inside the agent container. That single line is what ended the
//! drift where the desktop ran one Claude bridge and the image another:
//! there is no per-host spelling left to diverge. Adding a tool is one
//! row here plus installing its binary in the agent image — nothing else
//! (model roster and cost arrive from the agent over ACP at runtime).

/// Every fact the hosts need about one AI tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdapterSpec {
    /// THE id, equal to the tool name; also the suffix of the canonical
    /// member (`ai:<adapter>@joy`).
    pub adapter: &'static str,
    /// First-generation ids that still appear in unmigrated project.yaml
    /// pins and environment lists. Resolved by [`by_adapter`], written by
    /// nobody.
    pub legacy_ids: &'static [&'static str],
    /// Human-facing name for pickers and cards.
    pub label: &'static str,
    /// The canonical member this tool acts as.
    pub member: &'static str,
    /// ONE argv prefix that starts the tool's ACP endpoint, identical on
    /// the desktop and in the agent container (e.g. `qwen --acp`).
    pub entrypoint: &'static str,
    /// Binary probed on the PATH for "is it installed here".
    pub probe: &'static str,
    /// The provider-key environment variable the tool reads, when the
    /// platform holds a key for it. None for tools that only ever carry
    /// their own login.
    pub key_env: Option<&'static str>,
    /// The environment variable selecting the model, for tools that take
    /// it via env rather than ACP session config.
    pub model_env: Option<&'static str>,
    /// The environment variable naming the tool's own state directory
    /// (session logs, caches). The host points it at a per-chat directory
    /// so two chats never share tool state — identically on the desktop
    /// and in the container. None for tools without one.
    pub state_env: Option<&'static str>,
    /// What to tell a person on whose machine the probe fails.
    pub install_hint: &'static str,
}

/// The product's tools. Test-only agents (the platform's ACP mock) are
/// NOT rows here — a test builds its own [`AdapterSpec`] value instead of
/// leaking into the product registry.
pub const ADAPTERS: &[AdapterSpec] = &[
    AdapterSpec {
        adapter: "vibe",
        legacy_ids: &["mistral-vibe"],
        label: "Mistral Vibe",
        member: "ai:vibe@joy",
        // vibe speaks ACP natively through vibe-acp, which ships with the
        // Vibe CLI (zed.dev/acp/agent/mistral-vibe).
        entrypoint: "vibe-acp",
        probe: "vibe-acp",
        key_env: Some("MISTRAL_API_KEY"),
        model_env: Some("VIBE_ACTIVE_MODEL"),
        state_env: Some("VIBE_HOME"),
        install_hint: "Install the Mistral Vibe CLI (it ships vibe-acp) and sign in there",
    },
    AdapterSpec {
        adapter: "claude",
        legacy_ids: &["claude-code"],
        label: "Claude Code",
        member: "ai:claude@joy",
        // The official ACP bridge (same org as codex-acp). One spelling
        // ended the era where the desktop npx-ran one bridge package and
        // the agent image shipped another (JI-017A-85).
        entrypoint: "claude-agent-acp",
        probe: "claude-agent-acp",
        key_env: Some("ANTHROPIC_API_KEY"),
        model_env: None,
        state_env: Some("CLAUDE_CONFIG_DIR"),
        install_hint:
            "npm i -g @agentclientprotocol/claude-agent-acp; Claude Code signs in inside the tool",
    },
    AdapterSpec {
        adapter: "qwen",
        legacy_ids: &["qwen-code"],
        label: "Qwen Code",
        member: "ai:qwen@joy",
        entrypoint: "qwen --acp",
        probe: "qwen",
        key_env: Some("OPENAI_API_KEY"),
        model_env: Some("OPENAI_MODEL"),
        state_env: Some("QWEN_DIR"),
        install_hint: "Install Qwen Code (npm i -g @qwen-code/qwen-code) and sign in there",
    },
];

/// Resolve an adapter id — current or legacy — to its row. This is the
/// ONE lookup every surface uses; a legacy id resolves to the same row as
/// its tool name, so unmigrated pins keep working while the silent
/// migration catches up.
pub fn by_adapter(id: &str) -> Option<&'static AdapterSpec> {
    ADAPTERS
        .iter()
        .find(|spec| spec.adapter == id || spec.legacy_ids.contains(&id))
}

/// The row acting as a given member (`ai:vibe@joy` -> vibe).
pub fn by_member(member: &str) -> Option<&'static AdapterSpec> {
    ADAPTERS.iter().find(|spec| spec.member == member)
}

/// The current id for a possibly legacy adapter string: what the silent
/// project.yaml migration writes, and what every new pin records.
pub fn canonical_adapter_id(id: &str) -> Option<&'static str> {
    by_adapter(id).map(|spec| spec.adapter)
}

/// Where the ACP process runs. The placement is the WHOLE difference
/// between the hosts at this layer: the same entrypoint, either spawned
/// on the local PATH or bridged into the project container over a
/// long-lived `docker exec`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// Spawn on the local PATH (desktop). Environment goes onto the
    /// process descriptor, so it never appears in a command line.
    Local,
    /// Bridge into a container (platform). Environment must travel as
    /// `--env` argv because docker exec has no other channel.
    Container {
        /// The container to exec into.
        name: String,
        /// Working directory inside the container (the repo checkout).
        workdir: String,
        /// KEY=VALUE pairs for the tool (provider key, model, mode…).
        env: Vec<(String, String)>,
    },
}

/// Build the argv that starts this adapter's ACP endpoint at the given
/// placement. Element zero is the program.
pub fn command(spec: &AdapterSpec, placement: &Placement) -> Vec<String> {
    let entry = spec.entrypoint.split_whitespace().map(str::to_string);
    match placement {
        Placement::Local => entry.collect(),
        Placement::Container { name, workdir, env } => {
            let mut argv = vec![
                "docker".to_string(),
                "exec".to_string(),
                "-i".to_string(),
                "-w".to_string(),
                workdir.clone(),
            ];
            for (key, value) in env {
                argv.push("--env".to_string());
                argv.push(format!("{key}={value}"));
            }
            argv.push(name.clone());
            argv.extend(entry);
            argv
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_named_after_its_tool() {
        for spec in ADAPTERS {
            assert_eq!(spec.member, format!("ai:{}@joy", spec.adapter));
            assert!(
                !spec.legacy_ids.contains(&spec.adapter),
                "{}: the current id must not double as a legacy id",
                spec.adapter
            );
        }
    }

    #[test]
    fn a_legacy_id_resolves_to_the_same_row_as_the_tool_name() {
        for (legacy, tool) in [
            ("mistral-vibe", "vibe"),
            ("claude-code", "claude"),
            ("qwen-code", "qwen"),
        ] {
            let via_legacy = by_adapter(legacy).expect(legacy);
            let via_tool = by_adapter(tool).expect(tool);
            assert!(std::ptr::eq(via_legacy, via_tool));
            assert_eq!(canonical_adapter_id(legacy), Some(tool));
        }
        assert_eq!(by_adapter("copilot"), None);
        assert_eq!(canonical_adapter_id("mock"), None);
    }

    #[test]
    fn the_member_lookup_matches_the_naming_rule() {
        assert_eq!(by_member("ai:vibe@joy").unwrap().adapter, "vibe");
        assert_eq!(by_member("ai:codex@joy"), None);
    }

    #[test]
    fn local_placement_is_the_bare_entrypoint() {
        let spec = by_adapter("qwen").unwrap();
        assert_eq!(command(spec, &Placement::Local), vec!["qwen", "--acp"]);
    }

    #[test]
    fn container_placement_is_the_same_entrypoint_behind_docker_exec() {
        let spec = by_adapter("vibe").unwrap();
        let placement = Placement::Container {
            name: "joyint-project-123".into(),
            workdir: "/work/repo".into(),
            env: vec![
                ("MISTRAL_API_KEY".into(), "k".into()),
                ("VIBE_ACTIVE_MODEL".into(), "m".into()),
            ],
        };
        assert_eq!(
            command(spec, &placement),
            vec![
                "docker",
                "exec",
                "-i",
                "-w",
                "/work/repo",
                "--env",
                "MISTRAL_API_KEY=k",
                "--env",
                "VIBE_ACTIVE_MODEL=m",
                "joyint-project-123",
                "vibe-acp",
            ]
        );
    }
}
