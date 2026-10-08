// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The level of a turn reaches the agent itself, on every turn
//! (JOY-0280-A5, pinned here by JOY-02C3-46).
//!
//! A chat turn carries the agent mode its interaction level derives. The
//! lane sets that mode on the ACP session before the prompt, and it does
//! so on EVERY turn, not only when the mode changes: an agent may leave a
//! mode on its own (plan mode ends itself), and a lane that only set it
//! on change would let the next turn run less restricted than its level.
//!
//! The agent here is a shell script speaking the four requests of a
//! turn; it writes down each mode it is set to.

#![cfg(all(unix, feature = "acp"))]

joy_test_env::isolate!();

use joy_ai::acp_lane::{LaneConfig, LaneSet, TurnRequest};
use joy_chat::model::agent_mode::from_level;
use joy_core::model::config::InteractionLevel;

fn turn(n: usize, mode: joy_chat::model::AgentMode) -> TurnRequest {
    TurnRequest {
        chat_id: "chat".into(),
        turn_id: format!("turn-{n}"),
        prompt_full: "hello".into(),
        prompt_delta: Some("hello".into()),
        mode,
        max_price_cents: 0,
        activity: None,
        present: None,
    }
}

#[test]
fn every_turn_sets_the_session_mode_its_level_means() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("modes.log");
    let agent = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake_agent.sh");
    let config = LaneConfig {
        command: format!("FAKE_AGENT_LOG={} sh {agent}", log.display()),
        adapter: "claude".into(),
        cwd: dir.path().to_path_buf(),
        client_name: "joy-test".into(),
        client_version: "1".into(),
        fresh_preamble: None,
        model: None,
        prepare: None,
    };

    // The levels of five turns in one chat. The last two repeat the one
    // before: the mode is set again all the same.
    let levels = [
        InteractionLevel::Proposing,
        InteractionLevel::Confirmed,
        InteractionLevel::Autonomous,
        InteractionLevel::Proposing,
        InteractionLevel::Proposing,
    ];

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let lanes: LaneSet<u32> = LaneSet::default();
        for (n, level) in levels.iter().enumerate() {
            lanes
                .turn(
                    1,
                    0,
                    &config,
                    turn(n, from_level(*level)),
                    std::time::Duration::from_secs(20),
                )
                .await
                .unwrap_or_else(|e| panic!("turn {n} at {level:?}: {e}"));
        }
    });

    let set: Vec<String> = std::fs::read_to_string(&log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(
        set,
        ["plan", "acceptEdits", "bypassPermissions", "plan", "plan"],
        "one session mode per turn, the one its level means"
    );
}
