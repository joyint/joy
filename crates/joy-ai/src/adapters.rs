// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! THE adapter registry (JI-017A-85, decided 2026-07-29): one row per AI
//! tool, holding every fact both hosts need — and nothing else. The row
//! is data, not behavior; the ACP runtime around it (lanes, sessions,
//! budgets, container health) lives with the hosts.
//!
//! The adapter id IS the tool name (JOY-0231-74): `vibe`, `claude`,
//! `qwen`. The provider-flavored ids of the first generation exist ONLY
//! inside the official silent project.yaml migration
//! (`m_2026_07_adapter_tool_names`), which rewrites recorded pins; the
//! registry itself knows exactly one spelling per tool.
//!
//! The entrypoint is ONE argv prefix, valid verbatim on the desktop PATH
//! and inside the agent container. That single line is what ended the
//! drift where the desktop ran one Claude bridge and the image another:
//! there is no per-host spelling left to diverge. Adding a tool is one
//! row here plus installing its binary in the agent image — nothing else
//! (model roster and cost arrive from the agent over ACP at runtime).
//!
//! A row carries a LIST of launches, not one line, because a vendor may
//! ship the same program under two commands: GitHub's Copilot CLI is
//! `copilot` when installed from npm and `gh copilot` when driven through
//! the GitHub CLI, and a person may have either or both. The list is
//! ordered and the FIRST launch is canonical: it is what the agent image
//! installs and therefore what the container always runs. Only a desktop,
//! where we meet a machine we did not build, walks the list. What we must
//! never do is reach past a launcher into the binary it manages — joy
//! runs `gh copilot` exactly as the person runs it in their console
//! (operator rule 2026-09-19), because a homegrown shortcut around a
//! vendor's launcher is precisely the drift this registry exists to end.

/// One way to start a tool's ACP endpoint on a machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Launch {
    /// ONE argv prefix that starts the ACP endpoint (e.g. `qwen --acp`).
    pub entrypoint: &'static str,
    /// The program to look for on the PATH before trying this launch.
    pub probe: &'static str,
    /// The argv that settles whether this launch can really run the tool
    /// here, for launchers where finding [`Self::probe`] on the PATH is
    /// no proof. Exit 0 means yes. `gh` is the case that demands it: it
    /// sits on virtually every CI runner and dev machine and says nothing
    /// about whether Copilot is installed behind it, so keying off the
    /// binary alone produced spurious `ai:copilot@joy` registrations.
    /// None where the probe IS the tool and finding it is the answer.
    pub verify: Option<&'static str>,
}

/// Every fact the hosts need about one AI tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdapterSpec {
    /// THE id, equal to the tool name; also the suffix of the canonical
    /// member (`ai:<adapter>@joy`).
    pub adapter: &'static str,
    /// Human-facing name for pickers and cards.
    pub label: &'static str,
    /// The canonical member this tool acts as.
    pub member: &'static str,
    /// How this tool is started, in order of preference and never empty.
    /// The first entry is CANONICAL: the agent image installs it, so the
    /// container runs it without asking. A desktop picks the first launch
    /// that works on the machine in front of it ([`Self::usable_launch`]).
    pub launches: &'static [Launch],
    /// The provider-key environment variable the tool reads, when the
    /// platform holds a key for it. None for tools that only ever carry
    /// their own login.
    pub key_env: Option<&'static str>,
    /// The environment variable selecting the model, for tools that take
    /// it via env rather than ACP session config.
    pub model_env: Option<&'static str>,
    /// The environment variable naming the tool's HOME directory. It
    /// holds session logs and caches — and, for every current tool, the
    /// person's own LOGIN (vibe: VIBE_HOME carries the subscription
    /// auth; claude: CLAUDE_CONFIG_DIR carries Claude Code's sign-in;
    /// qwen: QWEN_DIR likewise). The rule (JAPP-01A0-1C): only an
    /// ISOLATED host that provides credentials itself may point this
    /// elsewhere — the platform container does (fresh home, key injected
    /// via `key_env`). A desktop must NEVER redirect it: the spawned
    /// agent would lose the person's sign-in and fail with "missing API
    /// key" while their own CLI works right next to it.
    pub state_env: Option<&'static str>,
    /// What to tell a person on whose machine the probe fails: prose
    /// only, no command. A surface pairs it with [`Self::install_command`]
    /// so the command can be shown as something to copy rather than
    /// buried in a sentence (JAPP-0245-7A).
    pub install_hint: &'static str,
    /// The one command that installs it, when there is one. None where
    /// installing means downloading an application.
    pub install_command: Option<&'static str>,
    /// The TOOL behind the bridge, when bridge and tool are separate
    /// programs (JAPP-0084-7D): the desktop ships the bridge as a
    /// sidecar, so the tool is what still has to be installed and
    /// signed in on the machine, and the bridge is told where it is.
    /// None when the entrypoint IS the tool (vibe-acp ships with vibe,
    /// qwen speaks ACP itself).
    pub tool: Option<ToolBinary>,
}

impl AdapterSpec {
    /// The canonical launch: what the agent image installs and the
    /// container runs, and what a desktop falls back to naming when
    /// nothing on the machine works (so a card still has something to
    /// show and an install hint still has something to mean).
    pub fn canonical(&self) -> &'static Launch {
        self.launches
            .first()
            .expect("every registry row names at least one launch")
    }

    /// The canonical argv prefix (JI-017A-85's ONE entrypoint).
    pub fn entrypoint(&self) -> &'static str {
        self.canonical().entrypoint
    }

    /// The canonical probe binary.
    pub fn probe(&self) -> &'static str {
        self.canonical().probe
    }

    /// The first launch that works on THIS machine, or None when the tool
    /// is not installed here at all.
    ///
    /// Both checks are handed in, which keeps the decision a pure walk
    /// over the registry that a test can drive without a PATH or a
    /// process: `on_path` answers "is the program there", `verify` runs a
    /// launch's [`Launch::verify`] argv and answers "did it exit 0".
    /// `verify` is only ever consulted for a launch that names one, so a
    /// machine with the plain tool installed never pays for a subprocess.
    pub fn usable_launch(
        &self,
        on_path: impl Fn(&str) -> bool,
        verify: impl Fn(&str) -> bool,
    ) -> Option<&'static Launch> {
        self.launches
            .iter()
            .find(|launch| on_path(launch.probe) && launch.verify.map(&verify).unwrap_or(true))
    }
}

/// The tool a bridge drives, as the registry states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolBinary {
    /// Binary probed on the PATH for "is the tool installed here".
    pub binary: &'static str,
    /// The environment variable the bridge reads to find the tool, set
    /// to the resolved absolute path by a host that spawns locally.
    pub env: &'static str,
    /// What to tell a person on whose machine the tool is missing: prose
    /// only, like [`AdapterSpec::install_hint`].
    pub install_hint: &'static str,
    /// The one command that installs it, when there is one.
    pub install_command: Option<&'static str>,
}

/// The product's tools. Test-only agents (the platform's ACP mock) are
/// NOT rows here — a test builds its own [`AdapterSpec`] value instead of
/// leaking into the product registry.
pub const ADAPTERS: &[AdapterSpec] = &[
    AdapterSpec {
        adapter: "vibe",
        label: "Mistral Vibe",
        member: "ai:vibe@joy",
        // vibe speaks ACP natively through vibe-acp, which ships with the
        // Vibe CLI (zed.dev/acp/agent/mistral-vibe).
        launches: &[Launch {
            entrypoint: "vibe-acp",
            probe: "vibe-acp",
            verify: None,
        }],
        key_env: Some("MISTRAL_API_KEY"),
        model_env: Some("VIBE_ACTIVE_MODEL"),
        state_env: Some("VIBE_HOME"),
        install_hint: "Install the Mistral Vibe CLI, which ships vibe-acp, and sign in there.",
        install_command: None,
        tool: None,
    },
    AdapterSpec {
        adapter: "claude",
        // "Claude Agent", not "Claude Code": Anthropic's branding rules
        // for third-party products forbid "Claude Code" as a label and
        // name "Claude Agent" as the preferred form in a picker.
        label: "Claude Agent",
        member: "ai:claude@joy",
        // The official ACP bridge (same org as codex-acp). One spelling
        // ended the era where the desktop npx-ran one bridge package and
        // the agent image shipped another (JI-017A-85).
        launches: &[Launch {
            entrypoint: "claude-agent-acp",
            probe: "claude-agent-acp",
            verify: None,
        }],
        key_env: Some("ANTHROPIC_API_KEY"),
        model_env: None,
        state_env: Some("CLAUDE_CONFIG_DIR"),
        install_hint:
            "The ACP bridge is missing. Install it, then sign in inside Claude Code itself.",
        install_command: Some("npm i -g @agentclientprotocol/claude-agent-acp"),
        // The bridge embeds the Claude Agent SDK, which looks for Claude
        // Code by this variable; the desktop points it at the person's
        // own installation (their sign-in stays in the tool, JAPP-001D).
        tool: Some(ToolBinary {
            binary: "claude",
            env: "CLAUDE_CODE_EXECUTABLE",
            install_hint: "Install Claude Code from claude.ai/code and sign in there.",
            install_command: None,
        }),
    },
    AdapterSpec {
        adapter: "qwen",
        label: "Qwen Code",
        member: "ai:qwen@joy",
        launches: &[Launch {
            entrypoint: "qwen --acp",
            probe: "qwen",
            verify: None,
        }],
        key_env: Some("OPENAI_API_KEY"),
        model_env: Some("OPENAI_MODEL"),
        state_env: Some("QWEN_DIR"),
        install_hint: "Install Qwen Code, then sign in there.",
        install_command: Some("npm i -g @qwen-code/qwen-code"),
        tool: None,
    },
    AdapterSpec {
        adapter: "copilot",
        label: "GitHub Copilot",
        member: "ai:copilot@joy",
        // GitHub ships ONE Copilot CLI under two commands, and a person
        // may have either or both. `copilot` is canonical: it is what the
        // agent image installs, and `gh copilot` itself prefers a
        // `copilot` on the PATH over its own download — so trying it
        // first is not our ordering, it is GitHub's.
        //
        // `gh` alone proves nothing (see Launch::verify), so the second
        // launch earns its place by answering `--version`. Note the `--`:
        // gh parses leading flags itself and passes everything after the
        // separator to Copilot untouched.
        launches: &[
            Launch {
                entrypoint: "copilot --acp",
                probe: "copilot",
                verify: None,
            },
            Launch {
                entrypoint: "gh copilot -- --acp",
                probe: "gh",
                verify: Some("gh copilot -- --version"),
            },
        ],
        // The container has no `gh` login to borrow, so the platform
        // injects a token; on a desktop Copilot carries its own sign-in.
        // Fine-grained PAT with the "Copilot Requests" permission — a
        // classic `ghp_` token is rejected by Copilot CLI.
        key_env: Some("COPILOT_GITHUB_TOKEN"),
        // The model roster arrives over ACP session config, not env.
        model_env: None,
        state_env: Some("COPILOT_HOME"),
        install_hint: "Install GitHub Copilot CLI and sign in, or run `gh copilot` once to let the GitHub CLI install it.",
        install_command: Some("npm i -g @github/copilot"),
        // The entrypoint IS the tool: Copilot CLI speaks ACP itself.
        tool: None,
    },
];

/// Resolve an adapter id to its row. Exact match only: recorded pins are
/// kept current by the official silent project.yaml migration, so no
/// other spelling exists at runtime.
pub fn by_adapter(id: &str) -> Option<&'static AdapterSpec> {
    ADAPTERS.iter().find(|spec| spec.adapter == id)
}

/// The row acting as a given member (`ai:vibe@joy` -> vibe).
pub fn by_member(member: &str) -> Option<&'static AdapterSpec> {
    ADAPTERS.iter().find(|spec| spec.member == member)
}

/// The registered id for an adapter string: `Some` exactly for the
/// registry's tools, `None` for mocks and unknown values.
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

/// Build the argv that starts a launch's ACP endpoint at the given
/// placement. Element zero is the program.
pub fn command(launch: &Launch, placement: &Placement) -> Vec<String> {
    let entry = launch.entrypoint.split_whitespace().map(str::to_string);
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
        }
    }

    #[test]
    fn only_the_exact_tool_id_resolves() {
        assert_eq!(by_adapter("vibe").unwrap().adapter, "vibe");
        // first-generation spellings live only in the official migration
        assert_eq!(by_adapter("mistral-vibe"), None);
        assert_eq!(by_adapter("claude-code"), None);
        assert_eq!(by_adapter("qwen-code"), None);
        assert_eq!(by_adapter("github-copilot"), None);
        assert_eq!(canonical_adapter_id("mock"), None);
    }

    #[test]
    fn every_row_names_at_least_one_launch() {
        for spec in ADAPTERS {
            assert!(
                !spec.launches.is_empty(),
                "{} names no launch",
                spec.adapter
            );
            // the canonical entrypoint must start with its own probe, or
            // the container would install one program and run another
            assert_eq!(
                spec.entrypoint().split_whitespace().next(),
                Some(spec.probe()),
                "{}'s canonical launch probes a different program than it runs",
                spec.adapter
            );
        }
    }

    /// The whole point of the list: one tool, two commands, and the
    /// machine decides. `gh` being present must never be enough.
    #[test]
    fn copilot_picks_the_launch_the_machine_actually_has() {
        let copilot = by_adapter("copilot").unwrap();
        let never = |_: &str| false;
        let always = |_: &str| true;

        // the plain CLI is preferred, and costs no subprocess
        let both = copilot
            .usable_launch(
                |_| true,
                |_| panic!("must not verify when copilot is on the PATH"),
            )
            .unwrap();
        assert_eq!(both.entrypoint, "copilot --acp");

        // only gh: the verify decides
        let gh_only = |b: &str| b == "gh";
        assert_eq!(
            copilot
                .usable_launch(gh_only, |argv| argv == "gh copilot -- --version")
                .unwrap()
                .entrypoint,
            "gh copilot -- --acp"
        );
        // gh on the PATH without Copilot behind it is NOT the tool
        assert_eq!(copilot.usable_launch(gh_only, never), None);

        // nothing installed
        assert_eq!(copilot.usable_launch(never, always), None);
    }

    #[test]
    fn a_single_launch_tool_needs_no_verify() {
        let qwen = by_adapter("qwen").unwrap();
        assert_eq!(
            qwen.usable_launch(|b| b == "qwen", |_| panic!("qwen names no verify"))
                .unwrap()
                .entrypoint,
            "qwen --acp"
        );
        assert_eq!(qwen.usable_launch(|_| false, |_| true), None);
    }

    #[test]
    fn the_member_lookup_matches_the_naming_rule() {
        assert_eq!(by_member("ai:vibe@joy").unwrap().adapter, "vibe");
        assert_eq!(by_member("ai:codex@joy"), None);
    }

    #[test]
    fn local_placement_is_the_bare_entrypoint() {
        let spec = by_adapter("qwen").unwrap();
        assert_eq!(
            command(spec.canonical(), &Placement::Local),
            vec!["qwen", "--acp"]
        );
    }

    /// A launcher's own separator survives into the argv: `gh` must see
    /// `--` or it would eat `--acp` as a flag of its own.
    #[test]
    fn a_launcher_keeps_its_separator() {
        let copilot = by_adapter("copilot").unwrap();
        let gh = copilot.launches[1];
        assert_eq!(
            command(&gh, &Placement::Local),
            vec!["gh", "copilot", "--", "--acp"]
        );
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
            command(spec.canonical(), &placement),
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
