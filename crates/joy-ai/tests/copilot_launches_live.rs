//! LIVE check that every launch the registry names for Copilot really
//! speaks ACP through the PRODUCT's own spawn path — `AcpAgent::from_str`
//! over the registry's entrypoint string, the same code the desktop runs.
//!
//! Ignored by default: it starts the real Copilot CLI and needs a signed-in
//! machine and the network. Run it deliberately:
//!
//! ```sh
//! cargo test -p joy-ai --features acp --test copilot_launches_live -- --ignored --nocapture
//! ```
//!
//! It opens a session and reads the model roster. That costs no tokens
//! (no prompt is sent), which is why it is a model listing and not a turn:
//! it proves the argv parses, the process starts, the handshake completes
//! and a session opens, without spending anyone's credits.
//!
//! A launch whose program is absent is SKIPPED, not failed — a machine
//! with only one of the two commands installed is a supported machine, and
//! that is half the point of the launch list.

#![cfg(feature = "acp")]

use joy_ai::acp_lane::{list_models, LaneConfig};
use joy_ai::adapters;

fn on_path(binary: &str) -> bool {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|dir| dir.join(binary).is_file()))
        .unwrap_or(false)
}

fn lane(entrypoint: &str) -> LaneConfig {
    LaneConfig {
        command: entrypoint.to_string(),
        adapter: "copilot".to_string(),
        cwd: std::env::temp_dir(),
        client_name: "joy-live-test".to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        fresh_preamble: None,
        model: None,
        prepare: None,
    }
}

#[test]
#[ignore = "starts the real Copilot CLI; needs a signed-in machine"]
fn every_copilot_launch_opens_an_acp_session() {
    let spec = adapters::by_adapter("copilot").expect("copilot is a registry row");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");

    let mut checked = 0;
    for launch in spec.launches {
        if !on_path(launch.probe) {
            eprintln!("skip {}: {} not on PATH", launch.entrypoint, launch.probe);
            continue;
        }
        let models = runtime
            .block_on(list_models(
                &lane(launch.entrypoint),
                std::time::Duration::from_secs(90),
            ))
            .unwrap_or_else(|e| panic!("{} did not open an ACP session: {e}", launch.entrypoint));

        assert!(
            !models.agent_version.is_empty(),
            "{} answered the handshake without naming a version",
            launch.entrypoint
        );
        eprintln!(
            "ok {} -> Copilot {}, {} model(s), default {:?}",
            launch.entrypoint,
            models.agent_version,
            models.options.len(),
            models.current
        );
        checked += 1;
    }

    assert!(
        checked > 0,
        "neither Copilot command is installed, so this test proved nothing; \
         install `copilot` or the GitHub CLI and run it again"
    );
}
