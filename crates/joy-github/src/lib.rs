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

use joy_forge_net::forge::{Ctx, Forge, Listing, NewRepository, ReleaseRequest, Target};
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
}
