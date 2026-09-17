// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! The real `joy-forge` binary, driven the way joy drives it
//! (JOY-0298-E4, design D2.1, D2.2a, D2.5 and D2.8).
//!
//! Everything here runs the shipped connector as a process, through
//! joy-core's own runner, against an in process fake forge API. No real
//! network and no forge CLI is involved, and the PATH is emptied for
//! the duration so that a curl or a gh on the developer's machine
//! cannot answer for the connector by accident.

use std::path::{Path, PathBuf};
use std::sync::Once;

use joy_core::forge_plugins::{self, CallContext, CallerFacts, Target};
use joy_forge_net::fake::{FakeForge, Reply};

/// The connector as cargo built it for this test run.
const CONNECTOR: &str = env!("CARGO_BIN_EXE_joy-forge");

static SETUP: Once = Once::new();

/// One environment for the whole test binary: an empty PATH (so no
/// curl and no gh exists), a HOME and an XDG config directory the
/// connector reads `forges.yaml` from, and the connector's own
/// directory registered as the plugin directory.
fn setup() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    SETUP.call_once(|| {
        let dir = std::env::temp_dir().join(format!("joy-forge-connector-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("config/joy")).expect("the config directory");
        // A machine without curl and without gh: the acceptance of J2.
        std::env::set_var("PATH", "");
        std::env::set_var("HOME", &dir);
        std::env::set_var("XDG_CONFIG_HOME", dir.join("config"));
        // and no forge CLI configuration anywhere
        std::env::remove_var("GH_CONFIG_DIR");
        std::env::remove_var("GLAB_CONFIG_DIR");
        std::env::remove_var("TEA_CONFIG_DIR");
        forge_plugins::set_plugin_dirs(vec![connector_dir().to_path_buf()]);
        DIR.set(dir).expect("one setup");
    });
    DIR.get().expect("the setup ran").clone()
}

fn connector_dir() -> &'static Path {
    Path::new(CONNECTOR)
        .parent()
        .expect("the connector's directory")
}

/// Point one host at the fake, the way an operator's workstation image
/// does (D2.5).
///
/// The cases run in parallel and share one environment, so the entry is
/// APPENDED under a lock and written by rename: a case never sees half
/// a file, and every case brings a host of its own.
fn write_forges_yaml(dir: &Path, host: &str, kind: &str, base: &str) {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _held = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = dir.join("config/joy/forges.yaml");
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    text.push_str(&format!(
        "- host: {host}\n  kind: {kind}\n  api_base: {base}\n"
    ));
    let staging = path.with_extension("yaml.new");
    std::fs::write(&staging, &text).expect("write forges.yaml");
    std::fs::rename(&staging, &path).expect("put forges.yaml in place");
}

fn spec(id: &str) -> &'static forge_plugins::ForgePluginSpec {
    forge_plugins::by_id(id).expect("the registry row")
}

/// The handshake of D2.2a: the shipped binary answers protocol 2 and
/// names every forge it carries, and joy finds it beside itself.
#[test]
fn the_connector_answers_the_handshake_with_protocol_2() {
    setup();
    let resolved = forge_plugins::resolve_plugin(spec("github")).expect("the connector resolves");
    assert_eq!(resolved.protocol, forge_plugins::PROTOCOL);
    assert!(resolved.is_combined(), "{:?}", resolved.resolved_path);
    assert!(
        resolved
            .plugin_version
            .as_deref()
            .is_some_and(|v| v.starts_with("joy-forge ")),
        "{:?}",
        resolved.plugin_version
    );
}

/// J2's acceptance: `joy-forge github identity --remote <url>` answers
/// on a machine without curl. The PATH is empty here, so there is none.
#[test]
fn identity_answers_on_a_machine_without_curl() {
    let dir = setup();
    let fake = FakeForge::start(|call| {
        if call.path == "/user/emails" {
            return Reply::json(200, r#"[{"email":"alice@acme.test","verified":true}]"#);
        }
        Reply::not_found()
    });
    write_forges_yaml(&dir, "ghe-identity.test", "github", &fake.base());
    let ctx = CallContext::rootless().with_facts(CallerFacts {
        login: Some("alice".into()),
        token_env: Some("GH_TOKEN".into()),
        token_value: Some("gho_only-in-the-child".into()),
        ..CallerFacts::default()
    });
    let identity = forge_plugins::identity_full(
        spec("github"),
        Some(&Target::remote("https://ghe-identity.test/acme/demo.git")),
        &ctx,
    )
    .expect("the connector answers");
    assert!(identity.known);
    assert_eq!(identity.login.as_deref(), Some("alice"));
    assert_eq!(identity.emails, vec!["alice@acme.test".to_string()]);
    assert!(fake.saw("GET", "/user/emails"));
}

/// J2's acceptance: an internal host listed in `forges.yaml` is claimed
/// with no forge CLI installed (D2.5). The PATH is empty, so gh, glab
/// and tea do not exist at all.
#[test]
fn an_internal_host_in_forges_yaml_is_claimed_without_any_forge_cli() {
    let dir = setup();
    write_forges_yaml(
        &dir,
        "git.internal.test",
        "gitea",
        "https://git.internal.test/api/v1",
    );
    let ctx = CallContext::rootless();
    let target = Target::remote("git@git.internal.test:team/app.git");
    assert!(
        forge_plugins::claims_full(spec("gitea"), &target, &ctx).expect("the connector answers"),
        "the configured instance is claimed"
    );
    // and it stays the Gitea operator's host, not everybody's
    assert!(!forge_plugins::claims_full(spec("github"), &target, &ctx).expect("answers"));
    assert!(!forge_plugins::claims_full(
        spec("gitea"),
        &Target::remote("git@stranger.test:team/app.git"),
        &ctx
    )
    .expect("answers"));
}

/// The store verb over REST, with the size normalised to bytes (D2.4).
#[test]
fn store_answers_over_rest_and_carries_the_size_in_bytes() {
    let dir = setup();
    let fake = FakeForge::start(|call| {
        if call.path.contains("/contents/.joy/project.yaml") {
            return Reply::text(200, "name: Demo\n");
        }
        if call.path == "/repos/acme/demo" {
            return Reply::json(200, r#"{"size":2,"default_branch":"main"}"#);
        }
        Reply::not_found()
    });
    write_forges_yaml(&dir, "ghe-store.test", "github", &fake.base());
    let ctx = CallContext::rootless().with_facts(CallerFacts {
        token_env: Some("GH_TOKEN".into()),
        token_value: Some("gho_only-in-the-child".into()),
        ..CallerFacts::default()
    });
    let answer = forge_plugins::store_full(
        spec("github"),
        &Target::remote("https://ghe-store.test/acme/demo.git"),
        &ctx,
    )
    .expect("the connector answers");
    assert_eq!(
        answer,
        forge_plugins::StoreAnswer::Store {
            project_yaml: "name: Demo\n".into(),
            size_bytes: Some(2 * 1024),
        }
    );
}

/// J2's acceptance, end to end: the release verb publishes over REST
/// through the real binary, on a machine without curl, with the token
/// handed in the way `joy release publish` hands it in.
#[test]
fn a_release_is_published_end_to_end_over_rest() {
    let dir = setup();
    let fake = FakeForge::start(|call| match (call.method.as_str(), call.path.as_str()) {
        ("GET", "/repos/acme/demo/releases/tags/v0.2.0") => Reply::not_found(),
        ("POST", "/repos/acme/demo/releases") => Reply::json(
            201,
            r#"{"id":3,"html_url":"https://ghe.acme.test/acme/demo/releases/tag/v0.2.0"}"#,
        ),
        _ => Reply::not_found(),
    });
    write_forges_yaml(&dir, "ghe-release.test", "github", &fake.base());
    let notes = dir.join("notes.md");
    std::fs::write(&notes, "## Changes\n\nFixed the thing\n").expect("the notes file");
    let ctx = CallContext::rootless().with_facts(CallerFacts {
        token_env: Some("GH_TOKEN".into()),
        token_value: Some("gho_only-in-the-child".into()),
        ..CallerFacts::default()
    });
    let outcome = forge_plugins::release(
        spec("github"),
        Some(&Target::remote("https://ghe-release.test/acme/demo.git")),
        "v0.2.0",
        "v0.2.0 - Second",
        &notes,
        &ctx,
    )
    .expect("the release is published");
    assert!(!outcome.unsupported);
    assert_eq!(
        outcome.url.as_deref(),
        Some("https://ghe.acme.test/acme/demo/releases/tag/v0.2.0")
    );
    let post = fake
        .calls()
        .into_iter()
        .find(|call| call.method == "POST")
        .expect("the create call");
    let body = post.json().expect("a JSON body");
    assert_eq!(body["tag_name"], "v0.2.0");
    assert!(body["body"].as_str().unwrap().contains("Fixed the thing"));
}

/// J2's acceptance in full: `joy release publish` succeeds on a machine
/// without curl **with `GH_TOKEN` set**. The caller names no variable
/// (joy-core has no forge knowledge and therefore no variable name to
/// pass), the PATH is empty so there is no gh to spawn either, and the
/// token still reaches the forge in a header: the connector reads the
/// forge's own variable (D2.4's `env` source).
#[test]
fn a_release_is_published_with_only_the_forges_own_variable_set() {
    let dir = setup();
    let fake = FakeForge::start(|call| match (call.method.as_str(), call.path.as_str()) {
        ("GET", "/repos/example/demo/releases/tags/v0.3.0") => Reply::not_found(),
        ("POST", "/repos/example/demo/releases") => Reply::json(
            201,
            r#"{"id":4,"html_url":"https://github.com/example/demo/releases/tag/v0.3.0"}"#,
        ),
        _ => Reply::not_found(),
    });
    // github.com itself, because the variable a forge's tooling reads
    // is chosen per host: GH_TOKEN for github.com, the enterprise pair
    // for anything else.
    write_forges_yaml(&dir, "github.com", "github", &fake.base());
    let notes = dir.join("notes-env.md");
    std::fs::write(&notes, "## Changes\n\nFixed the thing\n").expect("the notes file");
    const TOKEN: &str = "gho_only-in-the-environment";
    std::env::set_var("GH_TOKEN", TOKEN);
    let outcome = forge_plugins::release(
        spec("github"),
        Some(&Target::remote("https://github.com/example/demo.git")),
        "v0.3.0",
        "v0.3.0 - Third",
        &notes,
        // no CallerFacts at all: exactly what `joy release publish`
        // builds
        &CallContext::rootless(),
    )
    .expect("the release is published");
    assert_eq!(
        outcome.url.as_deref(),
        Some("https://github.com/example/demo/releases/tag/v0.3.0")
    );
    let post = fake
        .calls()
        .into_iter()
        .find(|call| call.method == "POST")
        .expect("the create call");
    assert_eq!(
        post.authorization(),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    std::env::remove_var("GH_TOKEN");
}

/// J3's acceptance: `joy release publish` succeeds on a machine with
/// **neither gh nor curl**, which is the one J2 could not carry because
/// the connector had no credential of its own in wave 1.
///
/// Nothing is in the environment either: the host is an Enterprise
/// Server, whose variables (`GH_ENTERPRISE_TOKEN`,
/// `GITHUB_ENTERPRISE_TOKEN`) nothing sets, the PATH is empty, and the
/// caller names no variable. The only credential on the machine is the
/// connector's own entry, in the 0600 file of D2.6.
#[test]
fn a_release_is_published_from_the_connectors_own_credential_alone() {
    let dir = setup();
    let fake = FakeForge::start(|call| match (call.method.as_str(), call.path.as_str()) {
        ("GET", "/repos/acme/publish/releases/tags/v0.4.0") => Reply::not_found(),
        ("POST", "/repos/acme/publish/releases") => Reply::json(
            201,
            r#"{"id":5,"html_url":"https://ghe-publish.test/acme/publish/releases/tag/v0.4.0"}"#,
        ),
        _ => Reply::not_found(),
    });
    write_forges_yaml(&dir, "ghe-publish.test", "github", &fake.base());
    const TOKEN: &str = "gho_only-in-the-connectors-own-entry";
    write_own_credential(&dir, "ghe-publish.test", "scotty", TOKEN);
    let notes = dir.join("notes-publish.md");
    std::fs::write(&notes, "## Changes\n\nFixed the thing\n").expect("the notes file");
    let outcome = forge_plugins::release(
        spec("github"),
        Some(&Target::remote("https://ghe-publish.test/acme/publish.git")),
        "v0.4.0",
        "v0.4.0 - Fourth",
        &notes,
        // no CallerFacts at all: exactly what `joy release publish`
        // builds
        &CallContext::rootless(),
    )
    .expect("the release is published");
    assert_eq!(
        outcome.url.as_deref(),
        Some("https://ghe-publish.test/acme/publish/releases/tag/v0.4.0")
    );
    let post = fake
        .calls()
        .into_iter()
        .find(|call| call.method == "POST")
        .expect("the create call");
    assert_eq!(
        post.authorization(),
        Some(format!("Bearer {TOKEN}").as_str()),
        "the token came from the connector's own entry and travelled in a header"
    );
}

/// The `token` verb answers from that same entry, with the login it
/// belongs to and the step that chose it (D2.4, D4.1c).
#[test]
fn the_token_verb_answers_from_the_connectors_own_entry() {
    let dir = setup();
    const TOKEN: &str = "gho_the-entry-of-this-machine";
    write_own_credential(&dir, "ghe-token.test", "scotty", TOKEN);
    write_forges_yaml(
        &dir,
        "ghe-token.test",
        "github",
        "https://ghe-token.test/api/v3",
    );
    let answer = forge_plugins::token(
        spec("github"),
        &Target::host("ghe-token.test"),
        &CallContext::rootless(),
    )
    .expect("the connector answers");
    assert!(answer.known);
    assert_eq!(answer.token.as_deref(), Some(TOKEN));
    assert_eq!(answer.login.as_deref(), Some("scotty"));
    assert_eq!(answer.source.as_deref(), Some("file"));
    assert_eq!(answer.chose_by.as_deref(), Some("only"));
    assert_eq!(answer.username.as_deref(), Some("x-access-token"));
    assert_eq!(answer.scopes.as_deref(), Some("repo user:email"));

    // The same verb with a direction (D4.1c's step 4). One login holds
    // the host, so no probe runs and the answer is the same one; what
    // this proves is that the shipped binary takes the flag, because a
    // flag it did not know would be a usage error and no answer at all.
    let directed = forge_plugins::token_for(
        spec("github"),
        &Target::host("ghe-token.test"),
        forge_plugins::Access::Write,
        &CallContext::rootless(),
    )
    .expect("the connector answers a directed ask");
    assert!(directed.known);
    assert_eq!(directed.token.as_deref(), Some(TOKEN));
    assert_eq!(directed.chose_by.as_deref(), Some("only"));
}

/// The connector's own credential file (D2.6), written the way a
/// finished `login` would have written it. The cases share one config
/// directory, so the entry is merged under a lock.
fn write_own_credential(dir: &Path, host: &str, login: &str, token: &str) {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _held = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = dir.join("config/joy/forge-tokens.json");
    let mut file: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| serde_json::json!({ "hosts": {} }));
    file["hosts"][host][login] = serde_json::json!({
        "token": token,
        "login": login,
        "scopes": "repo user:email",
    });
    let staging = path.with_extension("json.new");
    std::fs::write(&staging, serde_json::to_string_pretty(&file).unwrap())
        .expect("write forge-tokens.json");
    std::fs::rename(&staging, &path).expect("put forge-tokens.json in place");
}

/// A forge with no release backend still answers `unsupported`, so
/// publish keeps its tag-only path instead of failing.
#[test]
fn a_forge_without_a_release_backend_says_so() {
    let dir = setup();
    let notes = dir.join("notes-gitlab.md");
    std::fs::write(&notes, "notes").expect("the notes file");
    let outcome = forge_plugins::release(
        spec("gitlab"),
        Some(&Target::remote("https://gitlab.com/acme/demo.git")),
        "v0.1.0",
        "v0.1.0",
        &notes,
        &CallContext::rootless(),
    )
    .expect("the connector answers");
    assert!(outcome.unsupported);
}

/// J2's acceptance: `ps` during any call shows no token. The token is
/// handed to the connector in an environment variable it is told the
/// NAME of, and it reaches the forge in a header.
#[cfg(unix)]
#[test]
fn no_call_ever_carries_a_token_in_its_argument_list() {
    use std::time::{Duration, Instant};

    let dir = setup();
    // The fake holds the first request open long enough for this test
    // to read the child's own argument list out of /proc.
    let fake = FakeForge::start(|_| {
        std::thread::sleep(Duration::from_millis(900));
        Reply::json(200, "[]")
    });
    write_forges_yaml(&dir, "ghe-argv.test", "github", &fake.base());
    const TOKEN: &str = "gho_this-must-never-be-in-argv";
    let mut child = joy_process::command(CONNECTOR)
        .args([
            "github",
            "identity",
            "--remote",
            "https://ghe-argv.test/acme/demo.git",
            "--login",
            "alice",
            "--token-env",
            "GH_TOKEN",
            "--host-kind",
            "background",
        ])
        .env("GH_TOKEN", TOKEN)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("the connector starts");

    // Poll until the argument list is the CONNECTOR's own and complete.
    // Right after the fork /proc holds the parent's argv, and a moment
    // later a half written one, so "not empty" is not the signal: the
    // last argument joy passes is, and it is read last.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut seen = String::new();
    while Instant::now() < deadline {
        if let Ok(raw) = std::fs::read(format!("/proc/{}/cmdline", child.id())) {
            let text = String::from_utf8_lossy(&raw).replace('\0', " ");
            if text.contains("--host-kind background") {
                seen = text;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let status = child.wait().expect("the connector exits");
    assert!(status.success(), "the connector answered");
    assert!(
        !seen.is_empty(),
        "the connector's own argument list was never readable"
    );
    assert!(
        seen.contains(CONNECTOR),
        "the process read was ours: {seen}"
    );
    assert!(
        seen.contains("--token-env GH_TOKEN"),
        "the VARIABLE travels in argv: {seen}"
    );
    assert!(
        !seen.contains(TOKEN),
        "the token must never be in argv: {seen}"
    );
    // and it did reach the forge, in a header
    let call = fake.calls().into_iter().next().expect("one request");
    assert_eq!(
        call.authorization(),
        Some(format!("Bearer {TOKEN}").as_str())
    );
}
