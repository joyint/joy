// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The scope sets per forge and per verb group (D2.7a), and the
//! `scope_missing` answer (D2.7c).
//!
//! The verbs fall into seven groups. `claims`, `resolve`, `web-url` and
//! `version` make no network call and need no scope at all, so they are
//! not in here.
//!
//! The point of the module is one sentence of D2.7c: before a verb the
//! granted set cannot carry, the connector answers locally instead of
//! spending a request and reporting the forge's refusal as "denied".
//! A read only member keeps every verb except push, `create-repository`
//! and `release`, and hears why in one sentence.

use serde_json::json;

/// The seven verb groups of D2.7a.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// A: identity, the verified addresses.
    Identity,
    /// B: repository facts (`store`, `files`).
    RepositoryFacts,
    /// C: git read over https.
    GitRead,
    /// D: git write over https.
    GitWrite,
    /// E: list repositories.
    ListRepositories,
    /// F: create a repository.
    CreateRepository,
    /// G: a release with assets.
    Release,
}

/// What one group needs on one forge: every alternative that satisfies
/// it. A granted set satisfies the group when it carries ANY of them.
pub fn needed(forge: &str, group: Group) -> &'static [&'static str] {
    match forge {
        // One set covers A to G on github.com and on GHES alike:
        // `repo user:email`. There is no read only private scope on
        // GitHub, which D2.7a states instead of promising otherwise.
        "github" => match group {
            Group::Identity => &["user:email", "user"],
            Group::CreateRepository => &["repo", "public_repo"],
            _ => &["repo"],
        },
        // `write_repository` "Uses Git-over-HTTP. Does not support API
        // authentication.", so everything that goes through the API
        // needs `read_api` or `api`, and the two write verbs need `api`.
        "gitlab" => match group {
            Group::Identity | Group::RepositoryFacts | Group::ListRepositories => {
                &["read_api", "api"]
            }
            Group::GitRead => &["read_repository", "write_repository"],
            Group::GitWrite => &["write_repository"],
            Group::CreateRepository | Group::Release => &["api"],
        },
        // Gitea and Forgejo scope per category and let the HTTP method
        // pick the level; `POST /user/repos` is checked twice, by the
        // /user group and by the route.
        "gitea" => match group {
            Group::Identity => &["read:user", "write:user"],
            Group::RepositoryFacts | Group::GitRead | Group::ListRepositories => {
                &["read:repository", "write:repository"]
            }
            Group::GitWrite | Group::Release => &["write:repository"],
            Group::CreateRepository => &["write:user"],
        },
        _ => &[],
    }
}

/// The second scope a group needs beside [`needed`], where a forge asks
/// for two at once. Only Gitea's `create-repository` does: the /user
/// group and the route are both checked.
pub fn also_needed(forge: &str, group: Group) -> &'static [&'static str] {
    match (forge, group) {
        ("gitea", Group::CreateRepository) => &["write:repository"],
        _ => &[],
    }
}

/// Which of the group's requirements the granted set does not satisfy.
/// Empty means the verb may be attempted.
pub fn missing(forge: &str, group: Group, granted: &[String]) -> Vec<String> {
    let mut missing = Vec::new();
    for requirement in [needed(forge, group), also_needed(forge, group)] {
        if requirement.is_empty() {
            continue;
        }
        let satisfied = requirement
            .iter()
            .any(|scope| granted.iter().any(|have| have == scope));
        if !satisfied {
            // The first alternative is the one to ask for: it is the
            // narrowest set that works.
            missing.push(requirement[0].to_string());
        }
    }
    missing
}

/// The local answer of D2.7c, on exit code 0. It is an answer and not a
/// failure, and the host renders it as one sentence with one button.
pub fn scope_missing(
    host: &str,
    verb: &str,
    needed: &[String],
    have: &[String],
) -> serde_json::Value {
    json!({
        "state": "scope_missing",
        "host": host,
        "verb": verb,
        "needed": needed,
        "have": have,
        "next": "sign in again with wider access",
    })
}

/// A granted scope string as the forges write it: GitHub separates with
/// commas, GitLab and Gitea with spaces.
pub fn parse_granted(raw: &str) -> Vec<String> {
    raw.split([',', ' '])
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(scopes: &str) -> Vec<String> {
        parse_granted(scopes)
    }

    /// The v2 contradiction D2.7a resolves: with the read write set,
    /// `create-repository` is answered locally with `scope_missing`
    /// naming `api`, and `joy forge login --for create` widens it.
    #[test]
    fn the_gitlab_read_write_set_reads_everything_and_cannot_create() {
        let granted = set("read_api write_repository");
        for group in [
            Group::Identity,
            Group::RepositoryFacts,
            Group::GitRead,
            Group::GitWrite,
            Group::ListRepositories,
        ] {
            assert!(
                missing("gitlab", group, &granted).is_empty(),
                "{group:?} should be covered"
            );
        }
        assert_eq!(
            missing("gitlab", Group::CreateRepository, &granted),
            vec!["api".to_string()]
        );
        assert_eq!(
            missing("gitlab", Group::Release, &granted),
            vec!["api".to_string()]
        );
    }

    #[test]
    fn the_gitlab_read_only_set_covers_a_b_c_and_e_and_nothing_else() {
        let granted = set("read_api read_repository");
        for group in [
            Group::Identity,
            Group::RepositoryFacts,
            Group::GitRead,
            Group::ListRepositories,
        ] {
            assert!(missing("gitlab", group, &granted).is_empty(), "{group:?}");
        }
        assert_eq!(
            missing("gitlab", Group::GitWrite, &granted),
            vec!["write_repository".to_string()]
        );
    }

    #[test]
    fn githubs_one_set_covers_everything_and_a_public_only_token_cannot_reach_a_private_repo() {
        let full = set("repo,user:email");
        for group in [
            Group::Identity,
            Group::RepositoryFacts,
            Group::GitRead,
            Group::GitWrite,
            Group::ListRepositories,
            Group::CreateRepository,
            Group::Release,
        ] {
            assert!(missing("github", group, &full).is_empty(), "{group:?}");
        }
        let public = set("public_repo,user:email");
        assert!(missing("github", Group::CreateRepository, &public).is_empty());
        assert_eq!(
            missing("github", Group::RepositoryFacts, &public),
            vec!["repo".to_string()]
        );
    }

    #[test]
    fn gitea_asks_for_both_categories_before_it_creates_a_repository() {
        let read_write = set("read:user write:repository");
        assert!(missing("gitea", Group::Release, &read_write).is_empty());
        assert_eq!(
            missing("gitea", Group::CreateRepository, &read_write),
            vec!["write:user".to_string()]
        );
        let create = set("write:user write:repository");
        assert!(missing("gitea", Group::CreateRepository, &create).is_empty());
    }

    #[test]
    fn the_scope_missing_answer_has_the_shape_of_d2_7c() {
        let answer = scope_missing(
            "gitlab.com",
            "create-repository",
            &["api".to_string()],
            &set("read_api write_repository"),
        );
        assert_eq!(answer["state"], "scope_missing");
        assert_eq!(answer["host"], "gitlab.com");
        assert_eq!(answer["verb"], "create-repository");
        assert_eq!(answer["needed"], json!(["api"]));
        assert_eq!(answer["have"], json!(["read_api", "write_repository"]));
        assert_eq!(answer["next"], "sign in again with wider access");
    }

    #[test]
    fn a_granted_set_parses_from_both_separators() {
        assert_eq!(
            parse_granted("repo, user:email"),
            vec!["repo", "user:email"]
        );
        assert_eq!(
            parse_granted("read_api write_repository"),
            vec!["read_api", "write_repository"]
        );
        assert!(parse_granted("").is_empty());
    }
}
