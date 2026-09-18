// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! The GitHub connector against an in process fake API (JOY-0298-E4,
//! design D2.8).
//!
//! No real network is touched: the fake listens on 127.0.0.1 and the
//! connector is pointed at it through `forges.yaml`'s `api_base`, which
//! is the same road an operator's GitHub Enterprise Server takes.
//!
//! What every case here proves is one of J2's acceptance criteria: the
//! API path has no curl and no gh in it, a GHES host asks its OWN
//! `/api/v3`, the token travels in a header, and the release verb runs
//! over REST end to end.

use joy_forge_net::auth::store::{Record, Vault};
use joy_forge_net::config::Instances;
use joy_forge_net::fake::{FakeForge, Reply};
use joy_forge_net::forge::{Ctx, Listing, NewRepository, ReleaseRequest, Target};

/// The environment variable a token travels in, per test file, so two
/// test binaries cannot collide over one name.
const TOKEN_VAR: &str = "JOY_TEST_GITHUB_TOKEN";
const TOKEN: &str = "gho_a-secret-nobody-may-see";

fn ctx(fake: &FakeForge, host: &str, token: bool) -> Ctx {
    let instances = Instances::from_text(&format!(
        "- host: {host}\n  kind: github\n  api_base: {}\n",
        fake.base()
    ))
    .expect("the instance file parses");
    let ctx = Ctx::bare(std::env::temp_dir()).with_instances(instances);
    if token {
        // Once for the whole test binary: the cases run in threads, and
        // one writer is one writer however many of them ask.
        static SET: std::sync::Once = std::sync::Once::new();
        SET.call_once(|| std::env::set_var(TOKEN_VAR, TOKEN));
        return ctx.with_token_env(TOKEN_VAR);
    }
    ctx
}

fn remote(host: &str) -> Target {
    Target::Remote(format!("https://{host}/acme/demo.git"))
}

/// The acceptance of J2: `identity` answers on a machine without curl,
/// and a GHES host asks its own `/api/v3/user/emails`, never
/// api.github.com.
#[test]
fn identity_reads_the_instances_own_addresses() {
    let fake = FakeForge::start(|call| match call.path {
        ref path if path.starts_with("/user/emails") => Reply::json(
            200,
            r#"[{"email":"alice@acme.test","verified":true},{"email":"old@acme.test","verified":false}]"#,
        ),
        _ => Reply::not_found(),
    });
    let host = "ghe.acme.test";
    let ctx = ctx(&fake, host, true).with_login("alice");
    let answer = joy_github::github::identity_answer(&remote(host), &ctx);
    assert_eq!(answer["known"], true);
    assert_eq!(answer["login"], "alice");
    assert_eq!(answer["emails"], serde_json::json!(["alice@acme.test"]));

    let calls = fake.calls();
    assert_eq!(calls.len(), 1, "one request and no more: {calls:?}");
    assert_eq!(calls[0].path, "/user/emails");
    // the token is a header, never an argument (D5)
    assert_eq!(
        calls[0].authorization(),
        Some(format!("Bearer {TOKEN}").as_str())
    );
}

/// The hardcoded base of D2.8, seen from the other side: without a
/// configured instance a GHES host still asks ITS OWN domain.
#[test]
fn a_ghes_host_never_asks_api_github_com() {
    let ctx = Ctx::bare(std::env::temp_dir());
    assert_eq!(
        joy_github::github::api_base("ghe.acme.test", &ctx),
        "https://ghe.acme.test/api/v3"
    );
    assert_eq!(
        joy_github::github::api_base("github.com", &ctx),
        "https://api.github.com"
    );
}

#[test]
fn store_reads_the_project_yaml_and_normalises_the_size_to_bytes() {
    let fake = FakeForge::start(|call| {
        if call.path.contains("/contents/.joy/project.yaml") {
            return Reply::text(200, "name: Demo\n");
        }
        if call.path == "/repos/acme/demo" {
            return Reply::json(200, r#"{"size": 12, "default_branch": "main"}"#);
        }
        Reply::not_found()
    });
    let host = "ghe.acme.test";
    let answer = joy_github::github::store_answer(&remote(host), &ctx(&fake, host, true));
    assert_eq!(answer["state"], "store");
    assert_eq!(answer["project_yaml"], "name: Demo\n");
    assert_eq!(answer["size_bytes"], 12 * 1024);
}

#[test]
fn files_lists_the_default_branch() {
    let fake = FakeForge::start(|call| {
        if call.path.starts_with("/repos/acme/demo/git/trees/HEAD") {
            return Reply::json(
                200,
                r#"{"tree":[{"path":"VISION.md","type":"blob"},{"path":"docs","type":"tree"}],"truncated":false}"#,
            );
        }
        Reply::not_found()
    });
    let host = "ghe.acme.test";
    let answer = joy_github::github::files_answer(&remote(host), &ctx(&fake, host, true));
    assert_eq!(answer["state"], "files");
    assert_eq!(answer["paths"], serde_json::json!(["VISION.md"]));
}

#[test]
fn repositories_pages_and_filters_by_the_query() {
    let fake = FakeForge::start(|call| {
        if call.path.starts_with("/user/repos") {
            return Reply::json(
                200,
                r#"[{"full_name":"acme/demo","name":"demo","private":true,
                     "clone_url":"https://ghe.acme.test/acme/demo.git",
                     "ssh_url":"git@ghe.acme.test:acme/demo.git",
                     "default_branch":"main","html_url":"https://ghe.acme.test/acme/demo"},
                    {"full_name":"acme/other","name":"other","private":false,
                     "clone_url":"https://ghe.acme.test/acme/other.git",
                     "ssh_url":"git@ghe.acme.test:acme/other.git",
                     "default_branch":"main","html_url":"https://ghe.acme.test/acme/other"}]"#,
            );
        }
        Reply::not_found()
    });
    let host = "ghe.acme.test";
    let listing = Listing {
        query: Some("demo".into()),
        limit: 200,
        page: None,
    };
    let answer = joy_github::github::repositories_answer(
        &Target::Host(host.into()),
        &listing,
        &ctx(&fake, host, true),
    );
    assert_eq!(answer["state"], "repositories");
    let repositories = answer["repositories"].as_array().unwrap();
    assert_eq!(repositories.len(), 1);
    assert_eq!(repositories[0]["full_name"], "acme/demo");
    assert_eq!(repositories[0]["private"], true);
    assert_eq!(answer["truncated"], false);
}

/// D2.4's paging, with nothing lost in between: the cursor names the
/// page that was NOT read, so two answers hold the forge's rows in
/// order, without a gap and without a repetition.
#[test]
fn repositories_page_by_page_lose_no_row() {
    let fake = FakeForge::start(|call| {
        if !call.path.starts_with("/user/repos") {
            return Reply::not_found();
        }
        let page: usize = call
            .path
            .split("&page=")
            .nth(1)
            .and_then(|rest| rest.split('&').next())
            .and_then(|number| number.parse().ok())
            .unwrap_or(1);
        let rows: Vec<String> = match page {
            1 => vec!["acme/one".into(), "acme/two".into()],
            2 => vec!["acme/three".into(), "acme/four".into()],
            _ => Vec::new(),
        };
        let body = rows
            .iter()
            .map(|name| format!(r#"{{"full_name":"{name}","name":"{name}","private":false}}"#))
            .collect::<Vec<_>>()
            .join(",");
        Reply::json(200, format!("[{body}]"))
    });
    let host = "ghe.acme.test";
    let ctx = ctx(&fake, host, true);
    let ask = |page: Option<String>| {
        joy_github::github::repositories_answer(
            &Target::Host(host.into()),
            &Listing {
                query: None,
                limit: 2,
                page,
            },
            &ctx,
        )
    };
    let names = |answer: &serde_json::Value| -> Vec<String> {
        answer["repositories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["full_name"].as_str().unwrap().to_string())
            .collect()
    };

    let first = ask(None);
    assert_eq!(names(&first), ["acme/one", "acme/two"]);
    assert_eq!(first["truncated"], true);
    let cursor = first["next"].as_str().expect("a cursor").to_string();

    let second = ask(Some(cursor));
    assert_eq!(names(&second), ["acme/three", "acme/four"]);

    // and the end of the list says so: no cursor, nothing cut off
    let third = ask(second["next"].as_str().map(str::to_string));
    assert!(names(&third).is_empty());
    assert_eq!(third["truncated"], false);
    assert_eq!(third["next"], serde_json::Value::Null);
}

/// A page the answer has no room for is left UNREAD, so the cursor
/// points at it and its rows arrive whole in the next answer. The
/// filter of `--query` is what makes the pages uneven.
#[test]
fn a_page_that_would_overflow_the_limit_is_left_for_the_cursor() {
    let fake = FakeForge::start(|call| {
        if !call.path.starts_with("/user/repos") {
            return Reply::not_found();
        }
        let page: usize = call
            .path
            .split("&page=")
            .nth(1)
            .and_then(|rest| rest.split('&').next())
            .and_then(|number| number.parse().ok())
            .unwrap_or(1);
        // page 1 holds one matching row among three, page 2 three
        let rows: Vec<&str> = match page {
            1 => vec!["acme/demo", "other/a", "other/b"],
            2 => vec!["acme/demo-two", "acme/demo-three", "acme/demo-four"],
            _ => Vec::new(),
        };
        let body = rows
            .iter()
            .map(|name| format!(r#"{{"full_name":"{name}","name":"{name}","private":false}}"#))
            .collect::<Vec<_>>()
            .join(",");
        Reply::json(200, format!("[{body}]"))
    });
    let host = "ghe.acme.test";
    let ctx = ctx(&fake, host, true);
    let listing = Listing {
        query: Some("demo".into()),
        limit: 3,
        page: None,
    };
    let answer =
        joy_github::github::repositories_answer(&Target::Host(host.into()), &listing, &ctx);
    let rows = answer["repositories"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["full_name"], "acme/demo");
    assert_eq!(answer["truncated"], true);
    // the cursor names the page that was not read, not the one after it
    assert_eq!(answer["next"], "2");
}

/// Without a credential there is no account whose repositories could be
/// listed, and the answer says so instead of guessing.
#[test]
fn repositories_without_a_credential_asks_for_a_sign_in() {
    let fake = FakeForge::start(|_| Reply::not_found());
    let host = "ghe.acme.test";
    let answer = joy_github::github::repositories_answer(
        &Target::Host(host.into()),
        &Listing::default(),
        &ctx(&fake, host, false),
    );
    assert_eq!(answer["state"], "needs_sign_in");
    assert!(fake.calls().is_empty());
}

#[test]
fn create_repository_posts_and_answers_with_the_clone_urls() {
    let fake = FakeForge::start(|call| match (call.method.as_str(), call.path.as_str()) {
        ("GET", "/user") => Reply::json(200, r#"{"login":"alice"}"#)
            .with_header("X-OAuth-Scopes", "repo, user:email"),
        ("POST", "/user/repos") => Reply::json(
            201,
            r#"{"clone_url":"https://ghe.acme.test/alice/fresh.git",
                "ssh_url":"git@ghe.acme.test:alice/fresh.git",
                "default_branch":"main","html_url":"https://ghe.acme.test/alice/fresh"}"#,
        ),
        _ => Reply::not_found(),
    });
    let host = "ghe.acme.test";
    let new = NewRepository {
        name: "fresh".into(),
        owner: None,
        private: true,
    };
    let answer = joy_github::github::create_repository_answer(
        &Target::Host(host.into()),
        &new,
        &ctx(&fake, host, true),
    );
    assert_eq!(answer["created"], true);
    assert_eq!(answer["clone_url"], "https://ghe.acme.test/alice/fresh.git");
    let post = fake
        .calls()
        .into_iter()
        .find(|call| call.method == "POST")
        .expect("the create call");
    assert_eq!(post.json().unwrap()["private"], true);
}

/// D2.7c's local pre check: a token whose granted set cannot create a
/// repository is answered without spending the write request.
#[test]
fn create_repository_answers_scope_missing_before_it_spends_a_request() {
    let fake = FakeForge::start(|call| match (call.method.as_str(), call.path.as_str()) {
        ("GET", "/user") => Reply::json(200, r#"{"login":"alice"}"#)
            .with_header("X-OAuth-Scopes", "gist, user:email"),
        _ => Reply::not_found(),
    });
    let host = "ghe.acme.test";
    let new = NewRepository {
        name: "fresh".into(),
        owner: None,
        private: false,
    };
    let answer = joy_github::github::create_repository_answer(
        &Target::Host(host.into()),
        &new,
        &ctx(&fake, host, true),
    );
    assert_eq!(answer["state"], "scope_missing");
    assert_eq!(answer["needed"], serde_json::json!(["repo"]));
    assert_eq!(answer["next"], "sign in again with wider access");
    assert!(
        !fake.calls().iter().any(|call| call.method == "POST"),
        "no write request is spent on a set that cannot carry it"
    );
}

/// The J2 acceptance in its own right: a release is published over REST,
/// with no gh and no curl anywhere in the path.
#[test]
fn release_creates_the_release_over_rest() {
    let fake = FakeForge::start(|call| match (call.method.as_str(), call.path.as_str()) {
        ("GET", "/repos/acme/demo/releases/tags/v0.1.0") => Reply::not_found(),
        ("POST", "/repos/acme/demo/releases") => Reply::json(
            201,
            r#"{"id":7,"html_url":"https://ghe.acme.test/acme/demo/releases/tag/v0.1.0",
                    "upload_url":"http://127.0.0.1:1/repos/acme/demo/releases/7/assets{?name,label}"}"#,
        ),
        _ => Reply::not_found(),
    });
    let host = "ghe.acme.test";
    let request = ReleaseRequest {
        tag: "v0.1.0".into(),
        title: "v0.1.0 - First".into(),
        notes: "## Changes\n\nFixed the thing\n".into(),
    };
    let answer =
        joy_github::github::release_answer(&remote(host), &request, &ctx(&fake, host, true))
            .expect("the release is published");
    assert_eq!(
        answer["url"],
        "https://ghe.acme.test/acme/demo/releases/tag/v0.1.0"
    );
    let post = fake
        .calls()
        .into_iter()
        .find(|call| call.method == "POST")
        .expect("the create call");
    let body = post.json().unwrap();
    assert_eq!(body["tag_name"], "v0.1.0");
    assert_eq!(body["name"], "v0.1.0 - First");
    assert!(body["body"].as_str().unwrap().contains("Fixed the thing"));
    assert_eq!(
        post.authorization(),
        Some(format!("Bearer {TOKEN}").as_str())
    );
}

/// JOY-0248-AE, now over REST: a release a tag-triggered workflow made
/// already keeps its URL and gets the notes prepended exactly once.
#[test]
fn release_prepends_the_notes_to_a_release_the_workflow_already_made() {
    use std::sync::{Arc, Mutex};
    let body = Arc::new(Mutex::new("## Install\nrun the installer".to_string()));
    let seen = body.clone();
    let fake = FakeForge::start(
        move |call| match (call.method.as_str(), call.path.as_str()) {
            ("GET", "/repos/acme/demo/releases/tags/v0.1.0") => Reply::json(
                200,
                serde_json::json!({
                    "id": 7,
                    "html_url": "https://ghe.acme.test/acme/demo/releases/tag/v0.1.0",
                    "body": *seen.lock().unwrap(),
                })
                .to_string(),
            ),
            ("PATCH", "/repos/acme/demo/releases/7") => {
                let notes = call.json().unwrap()["body"].as_str().unwrap().to_string();
                *seen.lock().unwrap() = notes;
                Reply::json(200, "{}")
            }
            _ => Reply::not_found(),
        },
    );
    let host = "ghe.acme.test";
    let request = ReleaseRequest {
        tag: "v0.1.0".into(),
        title: "v0.1.0 - First".into(),
        notes: "## Changes\n\nFixed the thing\n".into(),
    };
    let ctx = ctx(&fake, host, true);
    let answer = joy_github::github::release_answer(&remote(host), &request, &ctx)
        .expect("the release is completed");
    assert_eq!(
        answer["url"],
        "https://ghe.acme.test/acme/demo/releases/tag/v0.1.0"
    );
    let after = body.lock().unwrap().clone();
    assert!(after.contains("Fixed the thing"), "{after}");
    assert!(after.contains("## Install"), "{after}");
    assert!(
        after.find("Fixed the thing") < after.find("## Install"),
        "the changelog sits above the installer section: {after}"
    );

    // a second publish leaves the notes alone
    let before = after;
    joy_github::github::release_answer(&remote(host), &request, &ctx).expect("idempotent");
    assert_eq!(*body.lock().unwrap(), before);
    assert_eq!(
        fake.calls().iter().filter(|c| c.method == "PATCH").count(),
        1
    );
}

/// A release nobody made must not look like a release that was made.
#[test]
fn release_reports_a_refusal_instead_of_degrading() {
    let fake = FakeForge::start(|_| {
        Reply::json(
            403,
            r#"{"message":"Resource not accessible by integration"}"#,
        )
    });
    let host = "ghe.acme.test";
    let request = ReleaseRequest {
        tag: "v0.1.0".into(),
        title: "v0.1.0".into(),
        notes: "notes".into(),
    };
    let error =
        joy_github::github::release_answer(&remote(host), &request, &ctx(&fake, host, true))
            .expect_err("a refused release is an error");
    let text = error.to_string();
    assert!(text.contains("could not be read"), "{text}");
    assert!(text.contains("denied"), "{text}");
    assert!(!text.contains(TOKEN), "no token in an error text: {text}");
}

/// D2.7c, the cheap half: the set the forge granted is stored beside
/// the token (J3), so a verb that set cannot carry is refused locally,
/// without a single request, and never reported as `denied`.
#[test]
fn a_stored_public_only_set_refuses_a_private_repository_without_a_request() {
    let fake = FakeForge::start(|_| Reply::json(500, r#"{"message":"never asked"}"#));
    let host = "ghe-scope.test";
    let dir = tempfile::tempdir().expect("a sandbox");
    let instances = Instances::from_text(&format!(
        "- host: {host}\n  kind: github\n  api_base: {}\n",
        fake.base()
    ))
    .expect("the instance file parses");
    let ctx = Ctx::bare(std::env::temp_dir())
        .with_instances(instances)
        .with_vault(Vault::file_at(dir.path()))
        .with_state_dir(dir.path());
    ctx.vault()
        .put(
            host,
            &Record {
                token: "gho_public_only".into(),
                login: Some("scotty".into()),
                scopes: "public_repo user:email".into(),
                ..Record::default()
            },
        )
        .expect("the credential is stored");

    let private = NewRepository {
        name: "secret".into(),
        owner: None,
        private: true,
    };
    let answer =
        joy_github::github::create_repository_answer(&Target::Host(host.into()), &private, &ctx);
    assert_eq!(answer["state"], "scope_missing");
    assert_eq!(answer["verb"], "create-repository");
    assert_eq!(answer["needed"], serde_json::json!(["repo"]));
    assert_eq!(
        answer["have"],
        serde_json::json!(["public_repo", "user:email"])
    );
    assert_eq!(answer["next"], "sign in again with wider access");
    assert!(
        fake.calls().is_empty(),
        "the pre check spends no request at all"
    );
}

/// D2.7c and JOY-02A9-48: the GitHub connector tells the ORGANISATION's
/// wall from "this login is not the one", and it does it on the forge's
/// own evidence.
///
/// The cheap reading first: GitHub names the restriction in the body of
/// its 403, so the probe answers the wall without spending a second
/// request.
#[test]
fn a_403_naming_the_restriction_is_read_as_the_organisations_wall() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/repos/acme/demo" => Reply::json(
            403,
            r#"{"message":"Although you appear to have the correct authorization credentials, the `acme` organization has enabled OAuth App access restrictions."}"#,
        ),
        _ => Reply::not_found(),
    });
    let host = "ghe.acme.test";
    let ctx = ctx(&fake, host, true);

    let reach = joy_github::github::reaches_repo(host, "acme/demo", TOKEN, &ctx)
        .expect("the forge answered");

    assert!(reach.wall, "the restriction is named in the body");
    assert!(!reach.read && !reach.push);
    assert_eq!(fake.calls().len(), 1, "and nothing else is asked");
}

/// The paid reading, asked only where no login reached the repository:
/// the OWNER's own organisation record is refused with the restriction
/// named. That is the forge saying the wall exists, and it is the only
/// thing that may name one.
#[test]
fn the_wall_is_the_owner_record_refused_with_the_restriction() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/orgs/acme" => Reply::json(
            403,
            r#"{"message":"Although you appear to have the correct authorization credentials, the `acme` organization has enabled OAuth App access restrictions."}"#,
        ),
        _ => Reply::not_found(),
    });
    let host = "ghe.acme.test";
    let ctx = ctx(&fake, host, true);

    // The repository answers 404 and says nothing about a wall.
    let reach = joy_github::github::reaches_repo(host, "acme/demo", TOKEN, &ctx)
        .expect("the forge answered");
    assert!(!reach.wall && !reach.read);

    // The second question does.
    assert!(joy_github::github::organisation_wall(
        host,
        "acme/demo",
        TOKEN,
        &ctx
    ));
    // An owner whose record says nothing about a restriction is no
    // wall: the repository was renamed, deleted or never visible to
    // this login.
    assert!(!joy_github::github::organisation_wall(
        host,
        "someone-else/demo",
        TOKEN,
        &ctx
    ));
}

/// And the reading joy will NOT make (JOY-02A9-48): a person who
/// belongs to `acme` and asks for a repository of `acme` that does not
/// exist, was renamed, or is private beyond this token's scope, gets a
/// 404 and no wall. Membership says nothing about a restriction, and
/// sending that person to an owner with nothing to approve is the one
/// thing this question exists to prevent.
#[test]
fn belonging_to_the_owner_organisation_is_not_a_wall() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/user/orgs" => Reply::json(200, r#"[{"login":"acme"},{"login":"other"}]"#),
        _ => Reply::not_found(),
    });
    let host = "ghe.acme.test";
    let ctx = ctx(&fake, host, true);

    assert!(!joy_github::github::organisation_wall(
        host,
        "acme/widgts",
        TOKEN,
        &ctx
    ));
    let asked: Vec<String> = fake.calls().iter().map(|call| call.path.clone()).collect();
    assert_eq!(
        asked,
        vec!["/orgs/acme"],
        "and the organisation list is not even asked for: {asked:#?}"
    );
}
