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

use joy_forge_net::forge::{Ctx, Forge, Listing, NewRepository, ReleaseRequest, Target};
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
}
