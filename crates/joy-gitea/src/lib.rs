// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! joy-gitea: the Gitea and Forgejo forge connector (JOY-025B-F6, epic
//! JOY-0251-AA), a LIBRARY since JOY-0298-E4.
//!
//! All Gitea knowledge lives here: host matching (Gitea and Forgejo are
//! self-hosted with no canonical domain, so a host is claimed when tea
//! is signed in to it, when `forges.yaml` names it, or through the
//! project's `forge:` override), the noreply alias form
//! `<username>@noreply.<instance>`, tea's config, the instance's own
//! REST API.
//!
//! Gitea has no release backend in joy yet: `release` answers
//! `unsupported`, and `joy release publish` keeps its tag-only path.

pub mod gitea;

use joy_forge_net::auth::oauth::OAuth;
use joy_forge_net::auth::Purpose;
use joy_forge_net::forge::{
    Account, Ctx, Forge, Listing, NewRepository, Reach, ReleaseRequest, Target,
};
use serde_json::{json, Value};

/// The Gitea forge, as the dispatcher sees it.
pub struct Gitea;

/// The one instance the binaries hand to the dispatcher.
pub const FORGE: Gitea = Gitea;

impl Forge for Gitea {
    fn id(&self) -> &'static str {
        "gitea"
    }

    fn display(&self) -> &'static str {
        "Gitea"
    }

    fn claims(&self, host: &str, _ctx: &Ctx) -> bool {
        gitea::claims_host(host, &gitea::configured_hosts())
    }

    fn identity(&self, target: &Target, ctx: &Ctx) -> Value {
        gitea::identity_answer(target, ctx)
    }

    fn resolve(&self, email: &str) -> Value {
        gitea::resolve_answer(email)
    }

    fn store(&self, target: &Target, ctx: &Ctx) -> Value {
        gitea::store_answer(target, ctx)
    }

    fn files(&self, target: &Target, ctx: &Ctx) -> Value {
        gitea::files_answer(target, ctx)
    }

    fn repositories(&self, target: &Target, listing: &Listing, ctx: &Ctx) -> Value {
        gitea::repositories_answer(target, listing, ctx)
    }

    fn create_repository(&self, target: &Target, new: &NewRepository, ctx: &Ctx) -> Value {
        gitea::create_repository_answer(target, new, ctx)
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
        gitea::scopes_for(purpose)
    }

    fn oauth(&self, host: &str, purpose: Purpose, ctx: &Ctx) -> Option<OAuth> {
        gitea::oauth_for(host, purpose, ctx)
    }

    fn account(&self, host: &str, token: &str, ctx: &Ctx) -> Option<Account> {
        gitea::account_of(host, token, ctx)
    }

    fn reaches(&self, host: &str, repo_path: &str, token: &str, ctx: &Ctx) -> Option<Reach> {
        gitea::reaches_repo(host, repo_path, token, ctx)
    }

    fn web_url(&self, target: &Target, ctx: &Ctx) -> Value {
        joy_forge_net::auth::verbs::https_twin(target, ctx)
    }

    /// The Gitea family accepts this name beside a token in basic
    /// authentication, which is the same word the engine already uses.
    ///
    /// There is no `revoke` beside it: the family offers no token
    /// revocation endpoint an OAuth client may call, so `logout`
    /// removes the entry and answers `"revoked": false` rather than
    /// pretending. That is the trait's default.
    fn https_username(&self) -> &'static str {
        "oauth2"
    }

    fn foreign_cli(&self) -> &'static str {
        "tea"
    }

    fn foreign_logout_command(&self, host: &str) -> String {
        gitea::tea_logout_command(host)
    }

    fn foreign_logins(&self, host: &str) -> Vec<String> {
        gitea::tea_logins_for(host)
    }
}
