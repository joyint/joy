// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! The GitLab connector against an in process fake API (JOY-0298-E4,
//! design D2.7a, D2.7c and D2.8).
//!
//! The case that carries J2's acceptance is the last one: a token with
//! the read write set (`read_api write_repository`) answers `store`,
//! `files` and `repositories`, and answers `create-repository` with
//! `scope_missing` naming `api`. That is the resolution of the v2
//! contradiction: `write_repository` "Uses Git-over-HTTP. Does not
//! support API authentication.", so creating a project needs `api`.

use joy_forge_net::config::Instances;
use joy_forge_net::fake::{Call, FakeForge, Reply};
use joy_forge_net::forge::{Ctx, Listing, NewRepository, Target};

const TOKEN_VAR: &str = "JOY_TEST_GITLAB_TOKEN";
const TOKEN: &str = "glpat-a-secret-nobody-may-see";
const HOST: &str = "gitlab.acme.test";

fn ctx(fake: &FakeForge) -> Ctx {
    let instances = Instances::from_text(&format!(
        "- host: {HOST}\n  kind: gitlab\n  api_base: {}\n",
        fake.base()
    ))
    .expect("the instance file parses");
    static SET: std::sync::Once = std::sync::Once::new();
    SET.call_once(|| std::env::set_var(TOKEN_VAR, TOKEN));
    Ctx::bare(std::env::temp_dir())
        .with_instances(instances)
        .with_token_env(TOKEN_VAR)
}

fn remote() -> Target {
    Target::Remote(format!("https://{HOST}/group/sub/demo.git"))
}

/// The read write set of D2.7a, as the instance reports it.
fn read_write_token(call: &Call) -> Option<Reply> {
    (call.path == "/personal_access_tokens/self").then(|| {
        Reply::json(
            200,
            r#"{"id":1,"scopes":["read_api","write_repository"],"active":true}"#,
        )
    })
}

/// The first hardcoded base of D2.8: a self hosted instance is asked
/// about its own people, never gitlab.com.
#[test]
fn identity_asks_the_instance_and_not_gitlab_com() {
    let fake = FakeForge::start(|call| {
        if call.path == "/user/emails" {
            return Reply::json(200, r#"[{"email":"alice@acme.test"}]"#);
        }
        Reply::not_found()
    });
    let answer = joy_gitlab::gitlab::identity_answer(&remote(), &ctx(&fake).with_login("alice"));
    assert_eq!(answer["known"], true);
    assert_eq!(answer["emails"], serde_json::json!(["alice@acme.test"]));
    let calls = fake.calls();
    assert_eq!(calls[0].path, "/user/emails");
    assert_eq!(
        calls[0].authorization(),
        Some(format!("Bearer {TOKEN}").as_str())
    );
}

/// J2's acceptance, in one case: the read write set reads everything
/// and is told, locally, that creating needs `api`.
#[test]
fn the_read_write_set_reads_everything_and_cannot_create() {
    let fake = FakeForge::start(|call| {
        if let Some(reply) = read_write_token(call) {
            return reply;
        }
        let project = "/projects/group%2Fsub%2Fdemo";
        if call
            .path
            .starts_with(&format!("{project}/repository/files/"))
        {
            return Reply::text(200, "name: Demo\n");
        }
        if call.path.starts_with(&format!("{project}?statistics=true")) {
            return Reply::json(
                200,
                r#"{"default_branch":"main","statistics":{"repository_size":8192}}"#,
            );
        }
        if call.path.starts_with(&format!("{project}/repository/tree")) {
            return Reply::json(
                200,
                r#"[{"path":"VISION.md","type":"blob"},{"path":"docs","type":"tree"}]"#,
            );
        }
        if call.path.starts_with("/projects?membership=true") {
            return Reply::json(
                200,
                r#"[{"path_with_namespace":"group/sub/demo","path":"demo","visibility":"private",
                     "http_url_to_repo":"https://gitlab.acme.test/group/sub/demo.git",
                     "ssh_url_to_repo":"git@gitlab.acme.test:group/sub/demo.git",
                     "default_branch":"main","web_url":"https://gitlab.acme.test/group/sub/demo"}]"#,
            );
        }
        if call.method == "POST" && call.path == "/projects" {
            panic!("the local pre check must answer before a write request is spent");
        }
        Reply::not_found()
    });
    let ctx = ctx(&fake);

    // store: the file, plus the project record that carries the size
    let store = joy_gitlab::gitlab::store_answer(&remote(), &ctx);
    assert_eq!(store["state"], "store");
    assert_eq!(store["project_yaml"], "name: Demo\n");
    // GitLab answers `repository_size` in bytes already (D2.4)
    assert_eq!(store["size_bytes"], 8192);

    // files
    let files = joy_gitlab::gitlab::files_answer(&remote(), &ctx);
    assert_eq!(files["state"], "files");
    assert_eq!(files["paths"], serde_json::json!(["VISION.md"]));

    // repositories
    let listing = Listing {
        query: None,
        limit: 200,
        page: None,
    };
    let repositories =
        joy_gitlab::gitlab::repositories_answer(&Target::Host(HOST.into()), &listing, &ctx);
    assert_eq!(repositories["state"], "repositories");
    let rows = repositories["repositories"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["full_name"], "group/sub/demo");
    assert_eq!(rows[0]["private"], true);

    // create-repository: answered locally, naming `api`
    let new = NewRepository {
        name: "fresh".into(),
        owner: None,
        private: true,
    };
    let created =
        joy_gitlab::gitlab::create_repository_answer(&Target::Host(HOST.into()), &new, &ctx);
    assert_eq!(created["state"], "scope_missing");
    assert_eq!(created["verb"], "create-repository");
    assert_eq!(created["needed"], serde_json::json!(["api"]));
    assert_eq!(
        created["have"],
        serde_json::json!(["read_api", "write_repository"])
    );
    assert_eq!(created["next"], "sign in again with wider access");
}

/// With the full set the project is created and the connector answers
/// with the two clone URLs the caller needs to add a remote.
#[test]
fn the_full_set_creates_the_project() {
    let fake = FakeForge::start(|call| {
        if call.path == "/personal_access_tokens/self" {
            return Reply::json(200, r#"{"scopes":["api","write_repository"]}"#);
        }
        if call.method == "POST" && call.path == "/projects" {
            return Reply::json(
                201,
                r#"{"http_url_to_repo":"https://gitlab.acme.test/alice/fresh.git",
                    "ssh_url_to_repo":"git@gitlab.acme.test:alice/fresh.git",
                    "default_branch":"main","web_url":"https://gitlab.acme.test/alice/fresh"}"#,
            );
        }
        Reply::not_found()
    });
    let new = NewRepository {
        name: "fresh".into(),
        owner: None,
        private: true,
    };
    let created =
        joy_gitlab::gitlab::create_repository_answer(&Target::Host(HOST.into()), &new, &ctx(&fake));
    assert_eq!(created["created"], true);
    assert_eq!(
        created["clone_url"],
        "https://gitlab.acme.test/alice/fresh.git"
    );
    let post = fake
        .calls()
        .into_iter()
        .find(|call| call.method == "POST")
        .expect("the create call");
    assert_eq!(post.json().unwrap()["visibility"], "private");
}

/// D2.7c's 404 rule: "404 is `gone` only when the set contains
/// `read_api` or `api`". A token that holds `write_repository` alone
/// meets the API as an anonymous caller, so GitLab hides a private
/// project behind the same 404 a deleted one gets, and the person must
/// not be told their repository is gone.
#[test]
fn a_404_on_a_set_that_cannot_read_the_api_is_not_a_verdict() {
    let fake = FakeForge::start(|call| {
        if call.path == "/personal_access_tokens/self" {
            return Reply::json(200, r#"{"scopes":["write_repository"]}"#);
        }
        Reply::not_found()
    });
    let store = joy_gitlab::gitlab::store_answer(&remote(), &ctx(&fake));
    assert_eq!(store["state"], "unknown");
}

/// The same 404 IS the verdict once the set could have seen a private
/// project.
#[test]
fn a_404_on_a_set_that_reads_the_api_is_gone() {
    let fake = FakeForge::start(|call| {
        if let Some(reply) = read_write_token(call) {
            return reply;
        }
        Reply::not_found()
    });
    let store = joy_gitlab::gitlab::store_answer(&remote(), &ctx(&fake));
    assert_eq!(store["state"], "gone");
}

/// D2.7c: a 403 whose WWW-Authenticate names an insufficient scope is a
/// scope problem, and the answer says so instead of `denied`.
#[test]
fn a_refusal_the_pre_check_could_not_see_is_still_not_denied() {
    let fake = FakeForge::start(|call| {
        if call.path == "/personal_access_tokens/self" || call.path.ends_with("/oauth/token/info") {
            // an instance that will not say what the token may do
            return Reply::json(404, "{}");
        }
        if call.method == "POST" && call.path == "/projects" {
            return Reply::json(403, r#"{"message":"403 Forbidden"}"#).with_header(
                "WWW-Authenticate",
                r#"Bearer realm="GitLab", error="insufficient_scope""#,
            );
        }
        Reply::not_found()
    });
    let new = NewRepository {
        name: "fresh".into(),
        owner: None,
        private: false,
    };
    let created =
        joy_gitlab::gitlab::create_repository_answer(&Target::Host(HOST.into()), &new, &ctx(&fake));
    assert_eq!(created["state"], "scope_missing");
    assert_eq!(created["needed"], serde_json::json!(["api"]));
}
