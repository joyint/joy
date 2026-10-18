// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! One file per member under `.joy/members/` (JI-019D-46).
//!
//! project.yaml keeps the project and the list of member ids; who a
//! member is and what they may do is in `members/<id>.yaml`. The id is
//! the file name and appears nowhere inside the file. In an open project
//! nothing else carries it either: items and logs name a person by
//! address and an AI member by name. In an anonymous project the id is
//! what stands for a person everywhere, and their address is in the
//! encrypted `members.yaml` ([`crate::members_file`]).
//!
//! This module is the shape on disk and nothing else. Every reader and
//! writer goes through [`crate::store::load_project`] and
//! [`crate::store::save_project`] and sees the same member map as
//! before.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::JoyError;
use crate::member_id;
use crate::migrations::ai_member_name;
use crate::model::item::Capability;
use crate::model::project::{
    AiDelegationEntry, Granted, Member, MemberCapabilities, MemberFileShape, MemberKind, Origin,
};
use joy_model::InteractionLevel;

/// The directory under `.joy/` that holds the member files.
pub const MEMBERS_DIR: &str = "members";

/// The capabilities of a member as the file says them: `all`, or the
/// list. No setting hangs on a single capability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
enum Capabilities {
    All(AllMarker),
    List(Vec<Capability>),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum AllMarker {
    All,
}

/// A member file (format 4). The field order is the order in the file:
/// what the member is and who, then what they may do, then the keys
/// nobody edits by hand.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct MemberFile {
    /// Required: a file without it, or with a word joy does not know, is
    /// named in an error and not guessed at.
    #[serde(rename = "type")]
    kind: MemberKind,
    /// One key: the name the member goes by everywhere (a person's
    /// address, in an anonymous project the file id, an AI member's or a
    /// platform's name), and under it what proves who they are.
    identity: BTreeMap<String, Identity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    adapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    capabilities: Capabilities,
    /// An AI member's interaction level: the most it may do on its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    level: Option<InteractionLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    granted: Option<Granted>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    origin: Option<Origin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    members_wrap: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    crypt_wraps: BTreeMap<String, String>,
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "joy_model::migrations::ai_member_name::de_map"
    )]
    ai_delegations: BTreeMap<String, AiDelegationEntry>,
    /// When this file last changed: what the merge driver decides by
    /// when two people changed the same member.
    updated: DateTime<Utc>,
}

/// What proves who a member is, under the name they go by.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Identity {
    /// In an anonymous project: the tag a known address is checked
    /// against ([`member_id::email_tag`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    email_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    verify_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kdf_nonce: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seed_wrap: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seed_wrap_recovery: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    enrollment_verifier: Option<String>,
}

/// The directory of the member files of the project at `root`.
pub fn members_dir(root: &Path) -> PathBuf {
    crate::store::joy_dir(root).join(MEMBERS_DIR)
}

fn file_path(root: &Path, id: &str) -> PathBuf {
    members_dir(root).join(format!("{id}.yaml"))
}

fn to_file(key: &str, member: &Member, updated: DateTime<Utc>) -> MemberFile {
    let identity = Identity {
        email_tag: member.email_tag.clone(),
        verify_key: member.verify_key.clone(),
        kdf_nonce: member.kdf_nonce.clone(),
        seed_wrap: member.seed_wrap.clone(),
        seed_wrap_recovery: member.seed_wrap_recovery.clone(),
        enrollment_verifier: member.enrollment_verifier.clone(),
    };
    MemberFile {
        kind: member.kind,
        identity: BTreeMap::from([(key.to_string(), identity)]),
        adapter: member.adapter.clone(),
        model: member.model.clone(),
        capabilities: match &member.capabilities {
            MemberCapabilities::All => Capabilities::All(AllMarker::All),
            MemberCapabilities::Specific(map) => Capabilities::List(map.keys().copied().collect()),
        },
        level: member.interaction_level,
        granted: member.granted.clone(),
        origin: member.origin.clone(),
        members_wrap: member.members_wrap.clone(),
        crypt_wraps: member.crypt_wraps.clone(),
        ai_delegations: member.ai_delegations.clone(),
        updated,
    }
}

/// The member a file describes, and the key it has in the member map: the
/// one name under `identity`.
fn from_file(id: &str, file: MemberFile, path: &Path) -> Result<(String, Member), JoyError> {
    let mut named = file.identity.into_iter();
    let (Some((key, identity)), None) = (named.next(), named.next()) else {
        return Err(JoyError::Other(format!(
            "{}: `identity` names one member, by the name they go by",
            path.display()
        )));
    };
    let mut member = Member::of_kind(
        file.kind,
        match file.capabilities {
            Capabilities::All(_) => MemberCapabilities::All,
            Capabilities::List(list) => MemberCapabilities::Specific(
                list.into_iter().map(|c| (c, Default::default())).collect(),
            ),
        },
    );
    member.interaction_level = file.level;
    member.adapter = file.adapter;
    member.model = file.model;
    member.granted = file.granted;
    member.origin = file.origin;
    member.verify_key = identity.verify_key;
    member.kdf_nonce = identity.kdf_nonce;
    member.seed_wrap = identity.seed_wrap;
    member.seed_wrap_recovery = identity.seed_wrap_recovery;
    member.enrollment_verifier = identity.enrollment_verifier;
    member.email_tag = identity.email_tag;
    member.members_wrap = file.members_wrap;
    member.crypt_wraps = file.crypt_wraps;
    member.ai_delegations = file.ai_delegations;
    member.file_id = Some(id.to_string());
    Ok((ai_member_name::read_owned(key), member))
}

/// The member file `id` as it is on disk, in format 4 whatever its shape,
/// and whether its shape is that of format 4.
fn read_file(root: &Path, id: &str) -> Result<(MemberFile, bool), JoyError> {
    let path = file_path(root, id);
    let value: serde_yaml_ng::Value = crate::store::read_yaml(&path)?;
    let in_shape = value.get("identity").is_some() || value.get("type").is_some();
    let value = if in_shape {
        value
    } else {
        crate::migrations::m_2026_10_member_identity::from_flat(id, value)
    };
    let file = serde_yaml_ng::from_value(value).map_err(|e| JoyError::YamlParse {
        path: path.clone(),
        source: e,
    })?;
    Ok((file, in_shape))
}

/// Read the members whose ids project.yaml lists, in the shape
/// project.yaml says they have.
pub(crate) fn read(
    root: &Path,
    ids: &[String],
    shape: MemberFileShape,
) -> Result<BTreeMap<String, Member>, JoyError> {
    let mut members = BTreeMap::new();
    for id in ids {
        let path = file_path(root, id);
        let file = match shape {
            MemberFileShape::Identity => crate::store::read_yaml(&path)?,
            MemberFileShape::Flat => read_file(root, id)?.0,
        };
        let (key, member) = from_file(id, file, &path)?;
        members.insert(key, member);
    }
    Ok(members)
}

/// The id a member's file has: the one it was read from or registered
/// under, in an anonymous project the id that stands for the person, and
/// for a member that has none yet a fresh one.
fn file_id_of(key: &str, member: &Member) -> String {
    if let Some(id) = &member.file_id {
        return id.clone();
    }
    if member_id::is_opaque_member_id(key) {
        return key.to_string();
    }
    member_id::new_member_file_id()
}

/// Write the member files so that they say exactly `members` in `shape`,
/// and hand back the ids in the order project.yaml lists them. A file
/// whose member is unchanged and whose shape is `shape` already is left
/// alone, a file whose member is gone is removed.
pub(crate) fn write(
    root: &Path,
    members: &BTreeMap<String, Member>,
    shape: MemberFileShape,
) -> Result<Vec<String>, JoyError> {
    let dir = members_dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| JoyError::WriteFile {
        path: dir.clone(),
        source: e,
    })?;
    let now = Utc::now();
    let mut ids = Vec::with_capacity(members.len());
    for (key, member) in members {
        let id = file_id_of(key, member);
        let path = file_path(root, &id);
        let existing = read_file(root, &id).ok();
        let updated = existing.as_ref().map(|(f, _)| f.updated).unwrap_or(now);
        let unchanged = to_file(key, member, updated);
        let same_shape = existing
            .as_ref()
            .is_some_and(|(_, in_shape)| *in_shape == (shape == MemberFileShape::Identity));
        let same_member = existing.as_ref().map(|(f, _)| f) == Some(&unchanged);
        if !(same_shape && same_member) {
            // a file only brought into shape keeps the date of its member
            let file = to_file(key, member, if same_member { updated } else { now });
            match shape {
                MemberFileShape::Identity => crate::store::write_yaml(&path, &file)?,
                MemberFileShape::Flat => {
                    let flat = crate::migrations::m_2026_10_member_identity::to_flat(
                        serde_yaml_ng::to_value(&file)?,
                    )?;
                    crate::store::write_yaml(&path, &flat)?
                }
            }
        }
        ids.push(id);
    }
    ids.sort();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            let is_member_file = path.extension().is_some_and(|e| e == "yaml");
            if is_member_file && !ids.iter().any(|id| id == stem) {
                std::fs::remove_file(&path).map_err(|e| JoyError::WriteFile {
                    path: path.clone(),
                    source: e,
                })?;
            }
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::project::{MemberLayout, Project};
    use crate::store::{joy_dir, load_project, save_project, PROJECT_FILE};

    fn caps(list: &[Capability]) -> MemberCapabilities {
        MemberCapabilities::Specific(list.iter().map(|c| (*c, Default::default())).collect())
    }

    /// A project that keeps its members in files, with a founder, an AI
    /// member and a person who is only known by an opaque id.
    fn project(root: &Path) -> Project {
        std::fs::create_dir_all(joy_dir(root)).unwrap();
        let mut project = Project::new("Shop".into(), Some("SH".into()));
        project.set_member_layout(MemberLayout::Files);

        let mut founder = Member::person(MemberCapabilities::All);
        founder.verify_key = Some("aa".repeat(32));
        project
            .register_member("founder@example.com", founder)
            .unwrap();

        let mut claude = Member::ai(caps(&[Capability::Plan, Capability::Implement]));
        claude.adapter = Some("claude".into());
        claude.model = Some("opus".into());
        claude.interaction_level = Some(InteractionLevel::Confirmed);
        project.register_member("claude", claude).unwrap();
        project
    }

    fn files(root: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(members_dir(root))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_project_comes_back_from_its_member_files_as_it_went_in() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let before = project(root);
        save_project(root, &before).unwrap();

        let after = load_project(root).unwrap();
        assert_eq!(after.member_layout(), MemberLayout::Files);
        let keys: Vec<&String> = after.member_keys().collect();
        assert_eq!(keys, ["claude", "founder@example.com"]);

        let claude = after.member_by_key("claude").unwrap();
        assert_eq!(claude.adapter.as_deref(), Some("claude"));
        assert_eq!(claude.model.as_deref(), Some("opus"));
        assert_eq!(claude.interaction_level, Some(InteractionLevel::Confirmed));
        assert!(claude.has_capability(&Capability::Plan));
        assert!(claude.has_capability(&Capability::Implement));
        assert!(!claude.has_capability(&Capability::Review));

        let founder = after.member_by_email("founder@example.com").unwrap();
        assert_eq!(founder.capabilities, MemberCapabilities::All);
        assert_eq!(founder.verify_key, Some("aa".repeat(32)));
    }

    #[test]
    fn project_yaml_lists_the_ids_and_the_id_is_only_the_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        save_project(root, &project(root)).unwrap();

        let raw = std::fs::read_to_string(joy_dir(root).join(PROJECT_FILE)).unwrap();
        let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&raw).unwrap();
        let listed: Vec<String> =
            serde_yaml_ng::from_value(value.get("members").unwrap().clone()).unwrap();
        assert_eq!(listed.len(), 2);
        // nothing of a member is left in project.yaml
        assert!(!raw.contains("founder@example.com"), "{raw}");
        assert!(!raw.contains("claude"), "{raw}");

        let on_disk: Vec<String> = listed.iter().map(|id| format!("{id}.yaml")).collect();
        assert_eq!(files(root), on_disk);
        for id in &listed {
            assert!(member_id::is_opaque_member_id(id), "{id}");
            let body = std::fs::read_to_string(file_path(root, id)).unwrap();
            assert!(
                !body.contains(id.as_str()),
                "the id is the file name only: {body}"
            );
        }
    }

    #[test]
    fn a_member_file_reads_like_this() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        save_project(root, &project(root)).unwrap();
        let after = load_project(root).unwrap();
        let id = after
            .member_by_key("claude")
            .unwrap()
            .file_id
            .clone()
            .unwrap();
        let body = std::fs::read_to_string(file_path(root, &id)).unwrap();
        let without_date: String = body
            .lines()
            .filter(|l| !l.starts_with("updated:"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            without_date,
            "type: ai\n\
             identity:\n  \
               claude: {}\n\
             adapter: claude\n\
             model: opus\n\
             capabilities:\n\
             - plan\n\
             - implement\n\
             level: confirmed"
        );
        let founder_id = after
            .member_by_email("founder@example.com")
            .unwrap()
            .file_id
            .clone()
            .unwrap();
        let body = std::fs::read_to_string(file_path(root, &founder_id)).unwrap();
        assert!(
            body.starts_with(&format!(
                "type: person\nidentity:\n  founder@example.com:\n    verify_key: {}\ncapabilities: all\n",
                "aa".repeat(32)
            )),
            "{body}"
        );
    }

    /// A member file of format 4 says what its member is; joy names a
    /// file that does not, or says a word it does not know, and guesses
    /// nothing.
    #[test]
    fn a_member_file_without_a_type_or_with_an_unknown_one_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        save_project(root, &project(root)).unwrap();
        let loaded = load_project(root).unwrap();
        let id = loaded
            .member_by_key("claude")
            .unwrap()
            .file_id
            .clone()
            .unwrap();
        let path = file_path(root, &id);
        let body = std::fs::read_to_string(&path).unwrap();

        std::fs::write(&path, body.replace("type: ai\n", "")).unwrap();
        let err = load_project(root).unwrap_err().to_string();
        assert!(err.contains(&id) && err.contains("type"), "{err}");

        std::fs::write(&path, body.replace("type: ai\n", "type: robot\n")).unwrap();
        let err = load_project(root).unwrap_err().to_string();
        assert!(err.contains(&id) && err.contains("robot"), "{err}");
    }

    /// A platform member is read and written, and nothing is assigned to
    /// it yet.
    #[test]
    fn a_platform_member_is_kept_and_takes_no_work() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut project = project(root);
        project
            .register_member(
                "joyint.com",
                Member::of_kind(MemberKind::Platform, caps(&[Capability::Implement])),
            )
            .unwrap();
        save_project(root, &project).unwrap();

        let after = load_project(root).unwrap();
        assert_eq!(after.kind_of("joyint.com"), MemberKind::Platform);
        assert!(!after.is_ai("joyint.com"));
        let err = after.assignee("joyint.com").unwrap_err();
        assert!(err.contains("platform member"), "{err}");
    }

    /// A project of joy 0.23 is read and written in its own shape until a
    /// person says yes to the update: its files keep `email` and `name`,
    /// and project.yaml keeps the format it says.
    #[test]
    fn member_files_of_joy_0_23_stay_as_they_are_until_the_update() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut project = project(root);
        project.set_member_file_shape(MemberFileShape::Flat);
        save_project(root, &project).unwrap();
        let yaml = std::fs::read_to_string(joy_dir(root).join(PROJECT_FILE)).unwrap();
        assert!(yaml.starts_with("format: 2\n"), "{yaml}");

        let loaded = load_project(root).unwrap();
        assert_eq!(loaded.member_file_shape(), MemberFileShape::Flat);
        assert_eq!(loaded.kind_of("claude"), MemberKind::Ai);
        assert_eq!(loaded.kind_of("founder@example.com"), MemberKind::Person);
        let founder_id = loaded
            .member_by_email("founder@example.com")
            .unwrap()
            .file_id
            .clone()
            .unwrap();
        let before = std::fs::read_to_string(file_path(root, &founder_id)).unwrap();
        assert!(
            before.starts_with("email: founder@example.com\n"),
            "{before}"
        );

        // a change before the update is written the way it was
        let mut changed = loaded.clone();
        changed
            .member_by_email_mut("founder@example.com")
            .unwrap()
            .seed_wrap = Some("dd".repeat(40));
        save_project(root, &changed).unwrap();
        let after = std::fs::read_to_string(file_path(root, &founder_id)).unwrap();
        assert!(after.starts_with("email: founder@example.com\n"), "{after}");
        assert!(after.contains("seed_wrap_passphrase: "), "{after}");
        assert!(!after.contains("identity:"), "{after}");
        let yaml = std::fs::read_to_string(joy_dir(root).join(PROJECT_FILE)).unwrap();
        assert!(yaml.starts_with("format: 2\n"), "{yaml}");
    }

    /// The yes rewrites every file in format 4, keeps each member's date,
    /// and lifts project.yaml to 4 in the same write.
    #[test]
    fn the_update_brings_every_file_into_format_4_with_its_date() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut project = project(root);
        project.set_member_file_shape(MemberFileShape::Flat);
        save_project(root, &project).unwrap();
        let loaded = load_project(root).unwrap();
        let dates: Vec<String> = std::fs::read_dir(members_dir(root))
            .unwrap()
            .map(|e| {
                let body = std::fs::read_to_string(e.unwrap().path()).unwrap();
                body.lines()
                    .find(|l| l.starts_with("updated:"))
                    .unwrap()
                    .to_string()
            })
            .collect();

        std::thread::sleep(std::time::Duration::from_millis(5));
        let mut updated = loaded.clone();
        updated.set_member_file_shape(MemberFileShape::Identity);
        save_project(root, &updated).unwrap();

        let yaml = std::fs::read_to_string(joy_dir(root).join(PROJECT_FILE)).unwrap();
        assert!(yaml.starts_with("format: 4\n"), "{yaml}");
        for entry in std::fs::read_dir(members_dir(root)).unwrap() {
            let body = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            assert!(body.starts_with("type: "), "{body}");
            let date = body.lines().find(|l| l.starts_with("updated:")).unwrap();
            assert!(dates.iter().any(|d| d == date), "{date} kept: {dates:?}");
        }
        let again = load_project(root).unwrap();
        assert_eq!(again.member_file_shape(), MemberFileShape::Identity);
        assert_eq!(again.member_map(), loaded.member_map());
    }

    #[test]
    fn saving_again_leaves_an_unchanged_member_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        save_project(root, &project(root)).unwrap();
        let mut loaded = load_project(root).unwrap();
        let claude_id = loaded
            .member_by_key("claude")
            .unwrap()
            .file_id
            .clone()
            .unwrap();
        let founder_id = loaded
            .member_by_email("founder@example.com")
            .unwrap()
            .file_id
            .clone()
            .unwrap();
        let claude_before = std::fs::read_to_string(file_path(root, &claude_id)).unwrap();
        let founder_before = std::fs::read_to_string(file_path(root, &founder_id)).unwrap();

        // Change the founder only, a moment later.
        std::thread::sleep(std::time::Duration::from_millis(5));
        loaded
            .member_by_email_mut("founder@example.com")
            .unwrap()
            .kdf_nonce = Some("bb".repeat(32));
        save_project(root, &loaded).unwrap();

        assert_eq!(
            std::fs::read_to_string(file_path(root, &claude_id)).unwrap(),
            claude_before,
            "an unchanged member keeps its file, date included"
        );
        assert_ne!(
            std::fs::read_to_string(file_path(root, &founder_id)).unwrap(),
            founder_before
        );
        assert_eq!(files(root).len(), 2, "the same two files, none added");
    }

    #[test]
    fn a_removed_member_loses_its_file_and_its_place_in_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        save_project(root, &project(root)).unwrap();
        let mut loaded = load_project(root).unwrap();
        loaded.remove_member("claude").unwrap();
        save_project(root, &loaded).unwrap();

        assert_eq!(files(root).len(), 1);
        let after = load_project(root).unwrap();
        assert_eq!(after.member_count(), 1);
        assert!(after.member_by_key("claude").is_none());
    }

    /// What is staged is what is written: the list, a new file, and the
    /// removal of a file whose member is gone.
    #[test]
    fn the_member_files_are_staged_with_the_project_removals_included() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let repo = git2::Repository::init(root).unwrap();
        save_project(root, &project(root)).unwrap();
        let staged = |repo: &git2::Repository| -> Vec<String> {
            let mut index = repo.index().unwrap();
            index.read(true).unwrap();
            index
                .iter()
                .map(|e| String::from_utf8_lossy(&e.path).into_owned())
                .collect()
        };
        let first = staged(&repo);
        assert!(
            first.contains(&".joy/project.yaml".to_string()),
            "{first:?}"
        );
        assert_eq!(
            first
                .iter()
                .filter(|p| p.starts_with(".joy/members/"))
                .count(),
            2,
            "{first:?}"
        );

        let mut loaded = load_project(root).unwrap();
        loaded.remove_member("claude").unwrap();
        save_project(root, &loaded).unwrap();
        let second = staged(&repo);
        assert_eq!(
            second
                .iter()
                .filter(|p| p.starts_with(".joy/members/"))
                .count(),
            1,
            "{second:?}"
        );
    }

    #[test]
    fn a_person_known_by_an_opaque_id_keeps_it_as_the_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(joy_dir(root)).unwrap();
        let mut project = Project::new("Secret".into(), Some("SE".into()));
        project.set_member_layout(MemberLayout::Files);
        let mut members = BTreeMap::new();
        let mut person = Member::person(MemberCapabilities::All);
        person.email_tag = Some("cc".repeat(32));
        members.insert("m-abcdefghij".to_string(), person);
        project.replace_members(members);
        save_project(root, &project).unwrap();

        assert_eq!(files(root), ["m-abcdefghij.yaml"]);
        let body = std::fs::read_to_string(file_path(root, "m-abcdefghij")).unwrap();
        assert!(
            !body.contains("email:") && !body.contains("name:"),
            "{body}"
        );
        let after = load_project(root).unwrap();
        assert!(after.member_by_key("m-abcdefghij").is_some());
    }

    #[test]
    fn a_project_from_before_the_member_files_is_read_and_written_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(joy_dir(root)).unwrap();
        let mut project = Project::new("Old".into(), Some("OL".into()));
        project
            .register_member(
                "founder@example.com",
                Member::person(MemberCapabilities::All),
            )
            .unwrap();
        save_project(root, &project).unwrap();

        assert!(!members_dir(root).exists());
        let raw = std::fs::read_to_string(joy_dir(root).join(PROJECT_FILE)).unwrap();
        assert!(raw.contains("founder@example.com:"), "{raw}");
        let after = load_project(root).unwrap();
        assert_eq!(after.member_layout(), MemberLayout::InProject);
        assert_eq!(after, project);
    }
}
