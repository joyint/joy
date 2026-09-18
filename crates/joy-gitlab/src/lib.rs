// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! joy-gitlab: the GitLab forge connector (JOY-0255-B3, epic
//! JOY-0251-AA), a LIBRARY since JOY-0298-E4.
//!
//! All GitLab knowledge lives here: host matching (gitlab.com, plus
//! every host glab is signed in to and every host `forges.yaml` names),
//! the noreply alias form `<id>-<username>@users.noreply.gitlab.com`,
//! glab's config, the instance's own REST API.
//!
//! GitLab has no release backend in joy yet: `release` answers
//! `unsupported`, and `joy release publish` keeps its tag-only path.

pub mod gitlab;

use joy_forge_net::auth::oauth::OAuth;
use joy_forge_net::auth::store::Record;
use joy_forge_net::auth::Purpose;
use joy_forge_net::forge::{
    AccountAnswer, Ctx, Forge, Listing, NewRepository, Reach, ReleaseRequest, Target,
};
use serde_json::{json, Value};

/// The GitLab forge, as the dispatcher sees it.
pub struct GitLab;

/// The one instance the binaries hand to the dispatcher.
pub const FORGE: GitLab = GitLab;

impl Forge for GitLab {
    fn id(&self) -> &'static str {
        "gitlab"
    }

    fn display(&self) -> &'static str {
        "GitLab"
    }

    fn claims(&self, host: &str, _ctx: &Ctx) -> bool {
        gitlab::claims_host(host, &gitlab::configured_hosts())
    }

    fn identity(&self, target: &Target, ctx: &Ctx) -> Value {
        gitlab::identity_answer(target, ctx)
    }

    fn resolve(&self, email: &str) -> Value {
        gitlab::resolve_answer(email)
    }

    fn store(&self, target: &Target, ctx: &Ctx) -> Value {
        gitlab::store_answer(target, ctx)
    }

    fn files(&self, target: &Target, ctx: &Ctx) -> Value {
        gitlab::files_answer(target, ctx)
    }

    fn repositories(&self, target: &Target, listing: &Listing, ctx: &Ctx) -> Value {
        gitlab::repositories_answer(target, listing, ctx)
    }

    fn create_repository(&self, target: &Target, new: &NewRepository, ctx: &Ctx) -> Value {
        gitlab::create_repository_answer(target, new, ctx)
    }

    fn release(
        &self,
        _target: &Target,
        _request: &ReleaseRequest,
        _ctx: &Ctx,
    ) -> anyhow::Result<Value> {
        Ok(json!({ "unsupported": true }))
    }

    fn scopes(&self, purpose: Purpose) -> &'static str {
        gitlab::scopes_for(purpose)
    }

    fn oauth(&self, host: &str, purpose: Purpose, ctx: &Ctx) -> Option<OAuth> {
        gitlab::oauth_for(host, purpose, ctx)
    }

    fn account(&self, host: &str, token: &str, ctx: &Ctx) -> AccountAnswer {
        gitlab::account_of(host, token, ctx)
    }

    fn reaches(&self, host: &str, repo_path: &str, token: &str, ctx: &Ctx) -> Option<Reach> {
        gitlab::reaches_repo(host, repo_path, token, ctx)
    }

    fn web_url(&self, target: &Target, ctx: &Ctx) -> Value {
        joy_forge_net::auth::verbs::https_twin(target, ctx)
    }

    fn revoke(&self, host: &str, record: &Record, ctx: &Ctx) -> bool {
        gitlab::revoke_token(host, record, ctx)
    }

    /// GitLab requires this name beside a token in basic
    /// authentication, which is the same word the engine already uses.
    fn https_username(&self) -> &'static str {
        "oauth2"
    }

    fn foreign_cli(&self) -> &'static str {
        "glab"
    }

    fn foreign_logout_command(&self, host: &str) -> String {
        format!("glab auth logout --hostname {host}")
    }

    fn foreign_logins(&self, host: &str) -> Vec<String> {
        gitlab::glab_logins(host)
    }
}
