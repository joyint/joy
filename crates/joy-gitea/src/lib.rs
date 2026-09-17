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

use joy_forge_net::forge::{Ctx, Forge, Listing, NewRepository, ReleaseRequest, Target};
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
}
