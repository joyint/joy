// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

#![deny(clippy::all)]

pub mod auth;
pub mod capabilities;
pub mod commit_msg;
pub mod context;
pub mod crypt;
pub mod embedded;
pub mod error;
pub mod event_log;
pub mod filter;
pub mod forge_plugins;
pub mod fortune;
pub mod git_ops;
pub mod guard;
pub mod host;
pub mod identity;
pub mod init;
pub mod items;
pub mod member_id;
pub mod member_ref;
pub mod members_file;
pub mod merge;
pub mod migrations;
pub mod milestones;
pub mod model;
pub mod privacy;
pub mod project_meta;
pub mod releases;
pub mod security_md;
pub mod short_id;
pub mod store;
pub mod templates;
#[cfg(feature = "tutorial")]
pub mod tutorial;
pub mod update;
pub mod util;
pub mod vcs;
pub mod version_bump;
pub mod version_files;

/// The ONE process wide TLS trust decision (design D1.12).
///
/// There is no single CA setting in this stack: Linux verifies against
/// OpenSSL's default verify paths (which git2's own openssl-probe
/// fills, lib.rs:766-771), macOS against the system anchors plus the
/// keychain trust settings, Windows against the Windows certificate
/// store. A corporate CA installed with `update-ca-certificates`, with
/// Keychain Access or by group policy is therefore trusted with no joy
/// setting at all, and that is the normal path.
///
/// This is the one escape hatch beside it, for the workstation image
/// that carries its CA as a file: `ca_bundle` and `ca_dir` from
/// `forges.yaml` (D2.5) and `http.sslCAInfo` and `http.sslCAPath` from
/// git config, which libgit2 reads nothing of. It is Linux only,
/// because `GIT_OPT_SET_SSL_CERT_LOCATIONS` is compiled for OpenSSL and
/// mbedTLS alone (settings.c:207-223); on macOS and Windows every entry
/// is refused BY NAME with the sentence that names the store which
/// decides instead, rather than being ignored in silence.
///
/// It is process global and runs before the first contact: the engine
/// calls it on every entry into libgit2 (`vcs::forge::git_environment`)
/// and the `Once` makes every call after the first free.
pub fn apply_ca_locations() {
    static APPLIED: std::sync::Once = std::sync::Once::new();
    APPLIED.call_once(|| {
        match ca_locations() {
            vcs::proxy::CaDecision::Nothing => {}
            vcs::proxy::CaDecision::Apply(entries) => {
                for entry in entries {
                    // SAFETY: a libgit2 global option, set inside the
                    // `Once` above and never changed afterwards, before
                    // any contact of this process is opened.
                    let applied = unsafe {
                        match entry.kind {
                            vcs::proxy::CaKind::Bundle => {
                                git2::opts::set_ssl_cert_file(entry.value.as_str())
                            }
                            vcs::proxy::CaKind::Directory => {
                                git2::opts::set_ssl_cert_dir(entry.value.as_str())
                            }
                        }
                    };
                    match applied {
                        Ok(()) => tracing::info!(
                            key = %entry.key,
                            source = %entry.source,
                            path = %entry.value,
                            "certificate authority location applied"
                        ),
                        Err(e) => tracing::warn!(
                            key = %entry.key,
                            source = %entry.source,
                            path = %entry.value,
                            error = %e,
                            "certificate authority location could not be applied"
                        ),
                    }
                }
            }
            vcs::proxy::CaDecision::Refused(sentences) => {
                for sentence in sentences {
                    tracing::warn!("{sentence}");
                }
            }
        }
    });
}

/// What this machine's configuration says about certificate authority
/// locations, before anything is applied: the two `forges.yaml` keys of
/// D2.5, then the two git config keys of D1.12, judged against the
/// certificate store this build talks to.
pub fn ca_locations() -> vcs::proxy::CaDecision {
    let mut entries = vcs::proxy::forges_yaml_ca(&forges_file());
    if let Ok(config) = git2::Config::open_default() {
        entries.extend(vcs::proxy::git_config_ca(&config));
    }
    vcs::proxy::ca_decision(entries, vcs::proxy::TrustStore::of_this_build())
}

/// `~/.config/joy/forges.yaml` (Windows: `%APPDATA%\joy\forges.yaml`),
/// beside the personal config joy already keeps there (D2.5).
fn forges_file() -> std::path::PathBuf {
    let config = store::global_config_path();
    match config.parent() {
        Some(dir) => dir.join("forges.yaml"),
        None => std::path::PathBuf::from("forges.yaml"),
    }
}
