// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! joy-github: the GitHub forge connector (JOY-0254-3C, epic
//! JOY-0251-AA), a LIBRARY since JOY-0298-E4.
//!
//! One binary carries every forge (D2.1), so the GitHub knowledge is
//! linked into `joy-forge` instead of shipping as a process of its own.
//! The `joy-github` binary stays beside it as a PATH fallback for
//! `cargo install` users, for one deprecation window (D2.2a).
//!
//! Facts, in order of authority:
//! - handed-in caller facts (`--login/--user-id`, a multi-account
//!   host's session) win over anything discovered locally;
//! - gh's config names the signed-in login, offline;
//! - the instance's own REST API lists the account's verified
//!   addresses (best effort: without the user:email scope the list
//!   stays empty and the answer still names the login).

pub mod github;

use joy_forge_net::auth::oauth::OAuth;
use joy_forge_net::auth::store::Record;
use joy_forge_net::auth::Purpose;
use joy_forge_net::forge::{
    AccountAnswer, Ctx, Forge, Listing, NewRepository, Reach, ReleaseRequest, Target,
};
use serde_json::Value;

/// The GitHub forge, as the dispatcher sees it.
pub struct GitHub;

/// The one instance the binaries hand to the dispatcher.
pub const FORGE: GitHub = GitHub;

impl Forge for GitHub {
    fn id(&self) -> &'static str {
        "github"
    }

    fn display(&self) -> &'static str {
        "GitHub"
    }

    fn claims(&self, host: &str, _ctx: &Ctx) -> bool {
        github::claims_host(host, &github::configured_hosts())
    }

    fn identity(&self, target: &Target, ctx: &Ctx) -> Value {
        github::identity_answer(target, ctx)
    }

    fn resolve(&self, email: &str) -> Value {
        github::resolve_answer(email)
    }

    fn store(&self, target: &Target, ctx: &Ctx) -> Value {
        github::store_answer(target, ctx)
    }

    fn files(&self, target: &Target, ctx: &Ctx) -> Value {
        github::files_answer(target, ctx)
    }

    fn repositories(&self, target: &Target, listing: &Listing, ctx: &Ctx) -> Value {
        github::repositories_answer(target, listing, ctx)
    }

    fn create_repository(&self, target: &Target, new: &NewRepository, ctx: &Ctx) -> Value {
        github::create_repository_answer(target, new, ctx)
    }

    fn release(
        &self,
        target: &Target,
        request: &ReleaseRequest,
        ctx: &Ctx,
    ) -> anyhow::Result<Value> {
        github::release_answer(target, request, ctx)
    }

    /// One set covers every verb group on GitHub (D2.7a), so `--for`
    /// changes nothing here and the design says why: there is no read
    /// only private scope on GitHub.
    fn scopes(&self, _purpose: Purpose) -> &'static str {
        github::SCOPES
    }

    fn oauth(&self, host: &str, purpose: Purpose, ctx: &Ctx) -> Option<OAuth> {
        github::oauth_for(host, purpose, ctx)
    }

    fn account(&self, host: &str, token: &str, ctx: &Ctx) -> AccountAnswer {
        github::account_of(host, token, ctx)
    }

    fn reaches(&self, host: &str, repo_path: &str, token: &str, ctx: &Ctx) -> Option<Reach> {
        github::reaches_repo(host, repo_path, token, ctx)
    }

    fn org_approval_url(&self, host: &str, owner: &str) -> Option<String> {
        github::org_approval_page(host, owner)
    }

    fn org_wall(&self, host: &str, repo_path: &str, token: &str, ctx: &Ctx) -> bool {
        github::organisation_wall(host, repo_path, token, ctx)
    }

    fn web_url(&self, target: &Target, ctx: &Ctx) -> Value {
        joy_forge_net::auth::verbs::https_twin(target, ctx)
    }

    fn revoke(&self, host: &str, record: &Record, ctx: &Ctx) -> bool {
        github::revoke_token(host, record, ctx)
    }

    fn https_username(&self) -> &'static str {
        "x-access-token"
    }

    fn foreign_cli(&self) -> &'static str {
        "gh"
    }

    fn foreign_logout_command(&self, host: &str) -> String {
        format!("gh auth logout --hostname {host}")
    }

    fn foreign_logins(&self, host: &str) -> Vec<String> {
        github::gh_logins(host)
    }
}
