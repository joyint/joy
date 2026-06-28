// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Lint (JOY-01C8-2D / ADR-042): a field that names a project member in a
//! persisted model must be typed `MemberRef`, never a raw `String`.
//!
//! `MemberRef` binds the display guarantee to the type: an opaque member id
//! (anonymous mode) never reaches the terminal or `--json`; it resolves to a
//! name/e-mail or an auth request instead. That guarantee only holds where the
//! field actually IS a `MemberRef`. A new `closed_by: String` on an on-disk
//! struct would silently bypass it and could leak (or, on read, mis-handle) a
//! member identity.
//!
//! This is the display-direction counterpart to the lookup-direction guarantee,
//! which the compiler already enforces (the `Project` member map is private, so
//! no call site can key it by a raw e-mail). The compiler cannot force a new
//! struct field to be `MemberRef`, so this test does, for the model files that
//! are serialized into the `.joy` working tree.

use std::path::Path;

/// Field names that denote a project member (an identity) and therefore must be
/// a `MemberRef`. `_by`-suffixed names are covered by convention below; these
/// are the member-semantic names that do not end in `_by`.
const MEMBER_FIELDS: &[&str] = &[
    "author",
    "reporter",
    "attester",
    "actor",
    "committer",
    "owner",
    "member",
];

/// `_by`-suffixed fields name the acting member by convention, EXCEPT these
/// documented references to non-member entities (e.g. a superseding item id).
const BY_SUFFIX_EXCEPTIONS: &[&str] = &["replaced_by"];

/// Source files defining structs that are serialized into `.joy/`.
const MODEL_FILES: &[&str] = &[
    "src/model/item.rs",
    "src/model/project.rs",
    "src/model/release.rs",
    "src/event_log.rs",
];

#[test]
fn persisted_member_fields_are_member_refs() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut violations = Vec::new();

    for rel in MODEL_FILES {
        let path = crate_dir.join(rel);
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        for (idx, line) in src.lines().enumerate() {
            let Some((name, ty)) = parse_field(line.trim()) else {
                continue;
            };
            let is_member_field = MEMBER_FIELDS.contains(&name)
                || (name.ends_with("_by") && !BY_SUFFIX_EXCEPTIONS.contains(&name));
            if is_member_field && !ty.contains("MemberRef") {
                violations.push(format!(
                    "  {}:{}: field `{name}: {ty}` names a member but is not a MemberRef",
                    rel,
                    idx + 1
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "member-identity fields in persisted models must be typed MemberRef \
         (ADR-042), not raw strings. If a `_by` field genuinely references a \
         non-member entity, add it to BY_SUFFIX_EXCEPTIONS with a comment:\n{}",
        violations.join("\n")
    );
}

/// Parse a `pub name: Type,` field declaration. Returns `(name, type)` only for
/// lines that look like a public struct field. Requiring the `pub` prefix is
/// what separates a field *declaration* (`pub created_by: Option<MemberRef>,`)
/// from a struct-literal *initializer* (`created_by: None,`), so builder code,
/// doc comments, method signatures and match arms are ignored. Model fields are
/// `pub` by convention (they are serialized data structs).
fn parse_field(line: &str) -> Option<(&str, &str)> {
    let line = line.strip_prefix("pub ")?;
    let colon = line.find(':')?;
    let name = line[..colon].trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let ty = line[colon + 1..].trim().trim_end_matches(',').trim();
    if ty.is_empty() {
        return None;
    }
    Some((name, ty))
}
