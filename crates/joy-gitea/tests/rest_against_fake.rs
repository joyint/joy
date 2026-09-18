// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! The Gitea connector against an in process fake API (JOY-0298-E4,
//! design D2.5, D2.7c and D2.8).
//!
//! Gitea is the forge with no canonical host, so this file is also
//! where `forges.yaml` earns its place: an internal instance nobody
//! signed a CLI in to is claimed, asked and answered.

use joy_forge_net::config::Instances;
use joy_forge_net::fake::{FakeForge, Reply};
use joy_forge_net::forge::{Ctx, NewRepository, Target};

const TOKEN_VAR: &str = "JOY_TEST_GITEA_TOKEN";
const TOKEN: &str = "gta-a-secret-nobody-may-see";
const HOST: &str = "git.acme.test";

fn ctx(fake: &FakeForge) -> Ctx {
    let instances = Instances::from_text(&format!(
        "- host: {HOST}\n  kind: gitea\n  api_base: {}\n",
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
    Target::Remote(format!("https://{HOST}/acme/demo.git"))
}

/// Gitea's own scheme is `Authorization: token <t>`, and the size it
/// reports is KiB, which the protocol carries as bytes (D2.4).
#[test]
fn store_speaks_giteas_auth_scheme_and_normalises_the_size() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/repos/acme/demo/raw/.joy/project.yaml" => Reply::text(200, "name: Demo\n"),
        "/repos/acme/demo" => Reply::json(200, r#"{"size": 5, "default_branch": "main"}"#),
        _ => Reply::not_found(),
    });
    let answer = joy_gitea::gitea::store_answer(&remote(), &ctx(&fake));
    assert_eq!(answer["state"], "store");
    assert_eq!(answer["size_bytes"], 5 * 1024);
    let call = &fake.calls()[0];
    assert_eq!(
        call.authorization(),
        Some(format!("token {TOKEN}").as_str())
    );
}

/// D2.7c: the `required=` list of Gitea's own refusal is parsed into
/// `needed`, and the answer is never `denied`.
#[test]
fn a_scope_refusal_names_what_the_instance_asked_for() {
    let fake = FakeForge::start(|call| {
        if call.method == "POST" && call.path == "/user/repos" {
            return Reply::json(
                403,
                r#"{"message":"token does not have at least one of required scope(s), required=[write:repository], token scope=read:user"}"#,
            );
        }
        Reply::not_found()
    });
    let new = NewRepository {
        name: "fresh".into(),
        owner: None,
        private: false,
    };
    let answer =
        joy_gitea::gitea::create_repository_answer(&Target::Host(HOST.into()), &new, &ctx(&fake));
    assert_eq!(answer["state"], "scope_missing");
    assert_eq!(answer["verb"], "create-repository");
    // what the route demanded is what a person is asked to sign in with
    assert_eq!(answer["needed"], serde_json::json!(["write:repository"]));
    // and what they hold is what they hold, never the opposite
    assert_eq!(answer["have"], serde_json::json!(["read:user"]));
}

#[test]
fn create_repository_answers_with_the_clone_urls() {
    let fake = FakeForge::start(|call| {
        if call.method == "POST" && call.path == "/user/repos" {
            return Reply::json(
                201,
                r#"{"clone_url":"https://git.acme.test/alice/fresh.git",
                    "ssh_url":"git@git.acme.test:alice/fresh.git",
                    "default_branch":"main","html_url":"https://git.acme.test/alice/fresh"}"#,
            );
        }
        Reply::not_found()
    });
    let new = NewRepository {
        name: "fresh".into(),
        owner: None,
        private: true,
    };
    let answer =
        joy_gitea::gitea::create_repository_answer(&Target::Host(HOST.into()), &new, &ctx(&fake));
    assert_eq!(answer["created"], true);
    assert_eq!(answer["ssh_url"], "git@git.acme.test:alice/fresh.git");
}
