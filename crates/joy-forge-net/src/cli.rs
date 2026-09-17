// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The connector's command line: one parser for the combined binary and
//! for every legacy name (D2.1, D2.2, D2.2a).
//!
//! `joy-forge` carries every forge and takes the forge id as its first
//! argument; `joy-github`, `joy-gitlab` and `joy-gitea` carry one forge
//! each and take none. Both are this module with a different list, so
//! the protocol cannot drift between them.
//!
//! The `version` verb is asked of the BINARY and never of a forge
//! inside it, so it comes first and alone (D2.2a).

use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use serde_json::{json, Value};

use crate::config::Instances;
use crate::forge::{Ctx, Forge, HostKind, Listing, NewRepository, ReleaseRequest, Target};

/// The protocol this connector speaks (D2.2a).
pub const PROTOCOL: u32 = 2;

/// What the binary is called and which version it is, for the `version`
/// answer.
pub struct Manifest {
    pub name: &'static str,
    pub version: &'static str,
}

#[derive(Parser)]
#[command(disable_version_flag = true)]
struct Cli {
    /// Run as if started in <PATH> (parity with joy's -w).
    #[arg(short = 'w', long, global = true)]
    working_dir: Option<PathBuf>,
    /// Who is behind the calling process (D1.10).
    #[arg(long, global = true)]
    host_kind: Option<String>,
    /// The login this call is pinned to (D4.1c).
    #[arg(long, global = true)]
    login: Option<String>,
    /// The NAME of an environment variable holding a forge token. Never
    /// the token itself: it must not appear in a process list.
    #[arg(long, global = true)]
    token_env: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Which protocol does this binary speak, and which forges does it
    /// carry?
    Version,
    /// Does this remote or host belong to the forge?
    Claims {
        #[arg(long)]
        remote: Option<String>,
        #[arg(long)]
        host: Option<String>,
    },
    /// Who is ACTING on the forge?
    Identity {
        #[arg(long)]
        remote: Option<String>,
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        user_id: Option<String>,
    },
    /// Whose address is this? Pure: answered from the address alone.
    Resolve {
        #[arg(long)]
        email: String,
    },
    /// Does the repository hold a joy store, and may the caller create
    /// one?
    Store {
        #[arg(long)]
        remote: Option<String>,
        #[arg(long)]
        host: Option<String>,
    },
    /// Which files does the repository's default branch carry?
    Files {
        #[arg(long)]
        remote: Option<String>,
        #[arg(long)]
        host: Option<String>,
    },
    /// The repositories this account can reach.
    Repositories {
        #[arg(long)]
        remote: Option<String>,
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        page: Option<String>,
    },
    /// Create a repository on the forge.
    CreateRepository {
        #[arg(long)]
        remote: Option<String>,
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        name: String,
        #[arg(long)]
        owner: Option<String>,
        #[arg(long)]
        private: bool,
    },
    /// Create (or complete) the release for a tag.
    Release {
        #[arg(long)]
        remote: Option<String>,
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        tag: String,
        #[arg(long)]
        title: String,
        /// The release notes, passed as a file (they are multi-line and
        /// may be long).
        #[arg(long)]
        notes_file: PathBuf,
    },
}

/// Run the connector with this list of forges. Returns the process exit
/// code: 0 for every answer, non-zero only where a verb reports a
/// failure instead of degrading (D2.3, the `release` verb).
pub fn run(forges: &[&dyn Forge], manifest: &Manifest) -> i32 {
    run_from(forges, manifest, std::env::args_os())
}

/// [`run`] with an explicit argument list, for the tests.
pub fn run_from<I, T>(forges: &[&dyn Forge], manifest: &Manifest, args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let argv: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let (forge_id, argv) = split_forge_id(forges, argv);
    let cli = match Cli::try_parse_from(&argv) {
        Ok(cli) => cli,
        Err(error) => {
            // clap's own exit code and stream: usage on stderr with
            // code 2, which is also the protocol 1 detector of D2.2a.
            let _ = error.print();
            return error.exit_code();
        }
    };
    if matches!(cli.command, Command::Version) {
        println!("{}", version_answer(forges, manifest));
        return 0;
    }
    if let Some(dir) = &cli.working_dir {
        if let Err(error) = std::env::set_current_dir(dir) {
            eprintln!(
                "joy: {} is not a directory joy can enter: {error}",
                dir.display()
            );
            return 1;
        }
    }
    let forge = match pick(forges, forge_id.as_deref()) {
        Ok(forge) => forge,
        Err(message) => {
            eprintln!("{message}");
            return 2;
        }
    };
    let host_kind = match cli.host_kind.as_deref() {
        Some(raw) => match raw.parse::<HostKind>() {
            Ok(kind) => kind,
            Err(message) => {
                eprintln!("joy: {message}");
                return 2;
            }
        },
        None => HostKind::default(),
    };
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let user_id = match &cli.command {
        Command::Identity { user_id, .. } => user_id.clone(),
        _ => None,
    };
    let ctx = Ctx::new(host_kind, cli.login, user_id, cli.token_env, root);
    answer(forge, &cli.command, &ctx)
}

/// The verb, answered. One JSON object on stdout, and for `release` the
/// reason on stderr with a non-zero exit.
fn answer(forge: &dyn Forge, command: &Command, ctx: &Ctx) -> i32 {
    let value = match command {
        Command::Version => unreachable!("answered before the context is built"),
        Command::Claims { remote, host } => match target(remote.as_deref(), host.as_deref()) {
            Some(target) => json!({ "claims": claims(forge, &target, ctx) }),
            None => json!({ "claims": false }),
        },
        Command::Resolve { email } => forge.resolve(email),
        Command::Identity { remote, host, .. } => {
            let target = target(remote.as_deref(), host.as_deref())
                .unwrap_or_else(|| Target::Host(String::new()));
            forge.identity(&target, ctx)
        }
        Command::Store { remote, host } => match target(remote.as_deref(), host.as_deref()) {
            Some(target) => forge.store(&target, ctx),
            None => crate::forge::unknown_state(),
        },
        Command::Files { remote, host } => match target(remote.as_deref(), host.as_deref()) {
            Some(target) => forge.files(&target, ctx),
            None => crate::forge::unknown_state(),
        },
        Command::Repositories {
            remote,
            host,
            query,
            limit,
            page,
        } => {
            let listing = Listing {
                query: query.clone(),
                limit: limit.unwrap_or(crate::forge::DEFAULT_LIMIT),
                page: page.clone(),
            };
            match target(remote.as_deref(), host.as_deref()) {
                Some(target) => forge.repositories(&target, &listing, ctx),
                None => crate::forge::unknown_state(),
            }
        }
        Command::CreateRepository {
            remote,
            host,
            name,
            owner,
            private,
        } => {
            let new = NewRepository {
                name: name.clone(),
                owner: owner.clone(),
                private: *private,
            };
            match target(remote.as_deref(), host.as_deref()) {
                Some(target) => forge.create_repository(&target, &new, ctx),
                None => crate::forge::unknown_state(),
            }
        }
        Command::Release {
            remote,
            host,
            tag,
            title,
            notes_file,
        } => {
            let notes = match std::fs::read_to_string(notes_file) {
                Ok(notes) => notes,
                Err(error) => {
                    eprintln!(
                        "joy: the release notes at {} could not be read: {error}",
                        notes_file.display()
                    );
                    return 1;
                }
            };
            let request = ReleaseRequest {
                tag: tag.clone(),
                title: title.clone(),
                notes,
            };
            let target = target(remote.as_deref(), host.as_deref())
                .unwrap_or_else(|| Target::Host(String::new()));
            match forge.release(&target, &request, ctx) {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("{error}");
                    return 1;
                }
            }
        }
    };
    println!("{value}");
    0
}

/// Whether this forge answers for the target, over all three sources of
/// D2.5: the forge's own knowledge, the instances an operator
/// configured, and the project's `forge:` override.
fn claims(forge: &dyn Forge, target: &Target, ctx: &Ctx) -> bool {
    let Some(host) = target.host() else {
        return false;
    };
    if ctx.instances.claims(forge.id(), &host) {
        return true;
    }
    if ctx.project_forge.as_deref() == Some(forge.id()) {
        return true;
    }
    // A host another forge's operator configured belongs to that forge,
    // whatever this one's own rule would say.
    if ctx
        .instance(&host)
        .is_some_and(|entry| entry.kind != forge.id())
    {
        return false;
    }
    forge.claims(&host, ctx)
}

fn target(remote: Option<&str>, host: Option<&str>) -> Option<Target> {
    match (remote, host) {
        (Some(url), _) if !url.trim().is_empty() => Some(Target::Remote(url.to_string())),
        (_, Some(host)) if !host.trim().is_empty() => Some(Target::Host(host.to_string())),
        _ => None,
    }
}

/// The one object `version` answers (D2.2a).
fn version_answer(forges: &[&dyn Forge], manifest: &Manifest) -> Value {
    json!({
        "protocol": PROTOCOL,
        "plugin": format!("{} {}", manifest.name, manifest.version),
        "forges": forges.iter().map(|forge| forge.id()).collect::<Vec<_>>(),
    })
}

/// Take the forge id off the front where the binary carries more than
/// one forge. `version` is the binary's own question and never has one.
fn split_forge_id(forges: &[&dyn Forge], argv: Vec<OsString>) -> (Option<String>, Vec<OsString>) {
    if forges.len() < 2 || argv.len() < 2 {
        return (None, argv);
    }
    let candidate = argv[1].to_string_lossy().to_string();
    if !forges.iter().any(|forge| forge.id() == candidate) {
        return (None, argv);
    }
    let mut rest = argv;
    rest.remove(1);
    (Some(candidate), rest)
}

/// Which forge answers: the id from the argument list, or the only one
/// this binary carries.
fn pick<'a>(forges: &[&'a dyn Forge], id: Option<&str>) -> Result<&'a dyn Forge, String> {
    match id {
        Some(id) => forges
            .iter()
            .copied()
            .find(|forge| forge.id() == id)
            .ok_or_else(|| format!("joy: this connector does not carry the forge '{id}'")),
        None if forges.len() == 1 => Ok(forges[0]),
        None => Err(format!(
            "joy: name the forge first: one of {}",
            forges
                .iter()
                .map(|forge| forge.id())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// A context for a caller that drives a forge as a library (the tests).
pub fn library_ctx(root: impl Into<PathBuf>, instances: Instances) -> Ctx {
    Ctx::bare(root).with_instances(instances)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::{unknown, unknown_state};

    struct Stub(&'static str);

    impl Forge for Stub {
        fn id(&self) -> &'static str {
            self.0
        }
        fn display(&self) -> &'static str {
            "Stub"
        }
        fn claims(&self, host: &str, _ctx: &Ctx) -> bool {
            host == format!("{}.test", self.0)
        }
        fn identity(&self, _t: &Target, _c: &Ctx) -> Value {
            unknown()
        }
        fn resolve(&self, _email: &str) -> Value {
            unknown()
        }
        fn store(&self, _t: &Target, _c: &Ctx) -> Value {
            unknown_state()
        }
        fn files(&self, _t: &Target, _c: &Ctx) -> Value {
            unknown_state()
        }
        fn repositories(&self, _t: &Target, _l: &Listing, _c: &Ctx) -> Value {
            unknown_state()
        }
        fn create_repository(&self, _t: &Target, _n: &NewRepository, _c: &Ctx) -> Value {
            unknown_state()
        }
        fn release(&self, _t: &Target, _r: &ReleaseRequest, _c: &Ctx) -> anyhow::Result<Value> {
            Ok(json!({ "unsupported": true }))
        }
    }

    const MANIFEST: Manifest = Manifest {
        name: "joy-forge",
        version: "0.20.0",
    };

    #[test]
    fn the_version_answer_names_the_protocol_and_every_forge_the_file_carries() {
        let a = Stub("github");
        let b = Stub("gitlab");
        let forges: Vec<&dyn Forge> = vec![&a, &b];
        let answer = version_answer(&forges, &MANIFEST);
        assert_eq!(answer["protocol"], 2);
        assert_eq!(answer["plugin"], "joy-forge 0.20.0");
        assert_eq!(answer["forges"], json!(["github", "gitlab"]));
    }

    #[test]
    fn the_combined_binary_takes_the_forge_id_first_and_version_takes_none() {
        let a = Stub("github");
        let b = Stub("gitlab");
        let forges: Vec<&dyn Forge> = vec![&a, &b];
        let (id, rest) = split_forge_id(
            &forges,
            ["joy-forge", "github", "claims", "--remote", "x"]
                .iter()
                .map(OsString::from)
                .collect(),
        );
        assert_eq!(id.as_deref(), Some("github"));
        assert_eq!(rest[1], OsString::from("claims"));
        let (id, rest) = split_forge_id(
            &forges,
            ["joy-forge", "version"]
                .iter()
                .map(OsString::from)
                .collect(),
        );
        assert_eq!(id, None);
        assert_eq!(rest[1], OsString::from("version"));
    }

    #[test]
    fn a_legacy_binary_carries_one_forge_and_needs_no_id() {
        let only = Stub("github");
        let forges: Vec<&dyn Forge> = vec![&only];
        let (id, rest) = split_forge_id(
            &forges,
            ["joy-github", "claims", "--remote", "x"]
                .iter()
                .map(OsString::from)
                .collect(),
        );
        assert_eq!(id, None);
        assert_eq!(rest.len(), 4);
        assert_eq!(pick(&forges, None).unwrap().id(), "github");
        assert!(pick(&forges, Some("gitlab")).is_err());
    }

    /// D2.5: `claims` consults the forge, the configured instances and
    /// the project override, and a host configured for another forge is
    /// that forge's business.
    #[test]
    fn claims_consults_the_forge_the_instances_and_the_project_override() {
        let github = Stub("github");
        let ctx = Ctx::bare(std::env::temp_dir()).with_instances(
            Instances::from_text("- host: internal.example\n  kind: github\n").unwrap(),
        );
        assert!(claims(
            &github,
            &Target::Remote("git@github.test:o/r.git".into()),
            &ctx
        ));
        assert!(claims(
            &github,
            &Target::Remote("https://internal.example/o/r.git".into()),
            &ctx
        ));
        assert!(!claims(
            &github,
            &Target::Remote("https://stranger.example/o/r.git".into()),
            &ctx
        ));
        let gitlab = Stub("gitlab");
        assert!(!claims(
            &gitlab,
            &Target::Remote("https://internal.example/o/r.git".into()),
            &ctx
        ));
    }
}
