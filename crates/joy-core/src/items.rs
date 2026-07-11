// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::path::Path;

use crate::error::JoyError;
use crate::model::item::{item_filename, Item, ItemType};
use crate::store;

/// Whether an ID names a job item (`<ACRONYM>-JOB-xxxx[-YY]`). The ID
/// shape routes deterministically between `.joy/items/` and
/// `.joy/jobs/`; no lookup ever scans both directories. JOY-01FE-37.
pub fn is_job_id(id: &str) -> bool {
    id.to_uppercase().contains("-JOB-")
}

/// The storage directory for an item, routed by its type.
fn dir_for_type(root: &Path, item_type: &ItemType) -> std::path::PathBuf {
    let sub = if *item_type == ItemType::Job {
        store::JOBS_DIR
    } else {
        store::ITEMS_DIR
    };
    store::joy_dir(root).join(sub)
}

/// Lightweight placeholder for an encrypted item the caller cannot
/// decrypt. ID is read from the filename, zone from the JOYCRYPT magic
/// header; nothing is decrypted. Used by `joy ls` to render a `[Crypted
/// in zone <name>]` row instead of failing the whole listing. See
/// JOY-0174-D3.
#[derive(Debug, Clone)]
pub struct LockedItem {
    pub id: String,
    pub zone: String,
}

/// Load all items from `.joy/items/`, separating decryptable ones from
/// encrypted blobs the caller has no zone-key for. Plaintext items and
/// items whose zone key is currently active are returned as `Item`;
/// items in zones without an active key are returned as
/// [`LockedItem`] placeholders. See JOY-0174-D3.
pub fn load_items_with_locked(root: &Path) -> Result<(Vec<Item>, Vec<LockedItem>), JoyError> {
    let mut metas = list_item_metadata(root)?;
    metas.sort_by(|a, b| a.path.file_name().cmp(&b.path.file_name()));

    let mut items: Vec<Item> = Vec::new();
    let mut locked: Vec<LockedItem> = Vec::new();
    for meta in metas {
        if let Some(zone) = meta.encrypted_zone.as_deref() {
            if crate::crypt::active_zone_key(zone).is_none() {
                locked.push(LockedItem {
                    id: meta.id,
                    zone: zone.to_string(),
                });
                continue;
            }
        }
        let item: Item = store::read_yaml(&meta.path)?;
        items.push(item);
    }

    normalize_id_refs(&mut items);
    let milestone_ids: Vec<String> = crate::milestones::load_milestones(root)
        .map(|list| list.into_iter().map(|m| m.id).collect())
        .unwrap_or_default();
    normalize_milestone_refs(&mut items, &milestone_ids);

    Ok((items, locked))
}

/// Load all items from `.joy/items/`. Inaccessible-encrypted items are
/// silently skipped (the caller treats them as not present). For
/// surfacing locked-item placeholders, use
/// [`load_items_with_locked`].
pub fn load_items(root: &Path) -> Result<Vec<Item>, JoyError> {
    let (items, _) = load_items_with_locked(root)?;
    Ok(items)
}

/// Load all job items from `.joy/jobs/`. Jobs are deliberately absent
/// from [`load_items`]: default views never touch this directory, the
/// `-J` views and job-targeted lookups do. JOY-01FE-37.
pub fn load_jobs(root: &Path) -> Result<Vec<Item>, JoyError> {
    let mut metas = list_job_metadata(root)?;
    metas.sort_by(|a, b| a.path.file_name().cmp(&b.path.file_name()));
    let mut jobs: Vec<Item> = Vec::new();
    for meta in metas {
        if let Some(zone) = meta.encrypted_zone.as_deref() {
            if crate::crypt::active_zone_key(zone).is_none() {
                continue;
            }
        }
        let item: Item = store::read_yaml(&meta.path)?;
        jobs.push(item);
    }
    Ok(jobs)
}

/// Load the jobs whose scope contains `item_id` (current and past).
/// This is the one reverse lookup that scans `.joy/jobs/`: items carry
/// no job references so that creating a job never touches its targets.
pub fn jobs_for_item(root: &Path, item_id: &str) -> Result<Vec<Item>, JoyError> {
    let jobs = load_jobs(root)?;
    Ok(jobs
        .into_iter()
        .filter(|j| {
            j.job
                .as_ref()
                .is_some_and(|spec| spec.scope.iter().any(|s| s == item_id))
        })
        .collect())
}

/// Return the short form of a full item ID, or None if the ID is not
/// in the new ACRONYM-XXXX-YY shape (legacy four-hex-digit IDs and
/// non-item IDs like ACRONYM-MS-NN return None).
/// "JOY-0042-A3" -> Some("JOY-0042")
/// "JOY-0042"    -> None
/// "JOY-MS-01"   -> None
fn short_form(full_id: &str) -> Option<&str> {
    let last_dash = full_id.rfind('-')?;
    let suffix = &full_id[last_dash + 1..];
    if suffix.len() != 2 || u8::from_str_radix(suffix, 16).is_err() {
        return None;
    }
    let prefix = &full_id[..last_dash];
    let prev_dash = prefix.rfind('-')?;
    let middle = &prefix[prev_dash + 1..];
    if middle.len() == 4 && u16::from_str_radix(middle, 16).is_ok() {
        Some(prefix)
    } else {
        None
    }
}

/// Return the short form of a full milestone ID, or None if the ID
/// is not in the new ACRONYM-MS-NN-YY shape (legacy ACRONYM-MS-NN
/// IDs return None).
/// "JOY-MS-01-A1" -> Some("JOY-MS-01")
/// "JOY-MS-01"    -> None
/// "JOY-0042-A3"  -> None
fn milestone_short_form(full_id: &str) -> Option<&str> {
    let last_dash = full_id.rfind('-')?;
    let suffix = &full_id[last_dash + 1..];
    if suffix.len() != 2 || u8::from_str_radix(suffix, 16).is_err() {
        return None;
    }
    let prefix = &full_id[..last_dash];
    if prefix.contains("-MS-") {
        Some(prefix)
    } else {
        None
    }
}

/// Rewrite short-form milestone references in `milestone` to their
/// full form, using the supplied known milestone IDs. Ambiguous short
/// forms are left untouched.
fn normalize_milestone_refs(items: &mut [Item], milestone_ids: &[String]) {
    use std::collections::HashMap;
    let mut map: HashMap<String, Option<String>> = HashMap::new();
    for ms_id in milestone_ids {
        if let Some(short) = milestone_short_form(ms_id) {
            map.entry(short.to_string())
                .and_modify(|e| *e = None)
                .or_insert_with(|| Some(ms_id.clone()));
        }
    }
    for item in items.iter_mut() {
        if let Some(ms) = item.milestone.as_deref() {
            if let Some(Some(full)) = map.get(ms) {
                item.milestone = Some(full.clone());
            }
        }
    }
}

/// Rewrite short-form item ID references in `parent` and `deps` to
/// their full form, in place. Ambiguous short forms (multiple items
/// share the same prefix) are left untouched.
fn normalize_id_refs(items: &mut [Item]) {
    use std::collections::HashMap;
    let mut map: HashMap<String, Option<String>> = HashMap::new();
    for item in items.iter() {
        if let Some(short) = short_form(&item.id) {
            map.entry(short.to_string())
                .and_modify(|e| *e = None)
                .or_insert_with(|| Some(item.id.clone()));
        }
    }
    for item in items.iter_mut() {
        if let Some(p) = item.parent.as_deref() {
            if let Some(Some(full)) = map.get(p) {
                item.parent = Some(full.clone());
            }
        }
        for dep in &mut item.deps {
            if let Some(Some(full)) = map.get(dep.as_str()) {
                *dep = full.clone();
            }
        }
    }
}

/// Record an attribute-level mutation on an item. Sets `updated` /
/// `updated_by` for sort recency AND appends an entry to `history` for
/// the audit footer. Use this whenever you mutate an item attribute
/// (status, priority, deps, assignee, edit, ...). Do NOT use it for
/// comment add / edit / rm; those use `touch_for_comment_change`
/// instead, because per-comment audit lives on the comment itself.
pub fn touch_for_attribute_change(item: &mut Item, by: &str) {
    let now = chrono::Utc::now();
    item.updated = now;
    item.updated_by = Some(by.into());
    item.history
        .get_or_insert_with(Vec::new)
        .push(crate::model::item::UpdateEntry {
            date: now,
            by: by.into(),
        });
}

/// Bump an item's `updated` / `updated_by` for sort recency without
/// appending to its attribute history. Use this for comment add / edit
/// / rm: the item is touched but no attribute changed, so the audit
/// trail of comment activity lives on the comment itself (its `edits`
/// list, plus the item's `comments` Vec membership) rather than in the
/// item's attribute history.
pub fn touch_for_comment_change(item: &mut Item, by: &str) {
    let now = chrono::Utc::now();
    item.updated = now;
    item.updated_by = Some(by.into());
}

/// Save an item to .joy/items/{ID}-{slug}.yaml (job items go to
/// .joy/jobs/ instead).
pub fn save_item(root: &Path, item: &Item) -> Result<(), JoyError> {
    let dir = dir_for_type(root, &item.item_type);
    let filename = item_filename(&item.id, &item.title);
    let path = dir.join(&filename);
    write_item_file(&path, item)?;
    let sub = if item.item_type == ItemType::Job {
        store::JOBS_DIR
    } else {
        store::ITEMS_DIR
    };
    let rel = format!("{}/{}/{}", store::JOY_DIR, sub, filename);
    crate::git_ops::auto_git_add(root, &[&rel]);
    Ok(())
}

/// Write an item file, encrypting in place when `crypt_zone` is set.
/// Reads the active session's zone keys (set by joy-cli after
/// passphrase verification); without an active key for the zone the
/// write fails with `ZoneAccessDenied`. ADR-040.
fn write_item_file(path: &Path, item: &Item) -> Result<(), JoyError> {
    let yaml = serde_yaml_ng::to_string(item).map_err(JoyError::Yaml)?;
    let bytes = match item.crypt_zone.as_deref() {
        Some(zone) => {
            let zone_key =
                crate::crypt::active_zone_key(zone).ok_or_else(|| JoyError::ZoneAccessDenied {
                    zone: zone.to_string(),
                })?;
            crate::crypt::encrypt_blob(zone, &zone_key, yaml.as_bytes())
        }
        None => yaml.into_bytes(),
    };
    write_atomic(path, &bytes)
}

/// Lightweight item metadata available without authentication.
/// Walks `.joy/items/`, peeks each file: if it is a JOYCRYPT blob,
/// reads the zone name from the header without decrypting; if it is
/// plaintext YAML, parses just enough to extract the id and
/// crypt_zone fields. Used by `joy crypt status` / `joy crypt ls` /
/// `joy auth` to count and locate Crypt content without prompting
/// the user for a passphrase.
#[derive(Debug, Clone)]
pub struct ItemMeta {
    pub id: String,
    pub path: std::path::PathBuf,
    pub encrypted_zone: Option<String>,
    /// crypt_zone field as parsed from the plaintext YAML; only
    /// populated when the file is plaintext.
    pub plaintext_crypt_zone: Option<String>,
}

impl ItemMeta {
    /// The zone this item belongs to, regardless of whether it is
    /// currently encrypted on disk.
    pub fn zone(&self) -> Option<&str> {
        self.encrypted_zone
            .as_deref()
            .or(self.plaintext_crypt_zone.as_deref())
    }
}

/// Walk `.joy/items/` and return one `ItemMeta` per item file.
/// Never prompts, never decrypts. Use `load_items` when you need
/// full Item objects.
pub fn list_item_metadata(root: &Path) -> Result<Vec<ItemMeta>, JoyError> {
    list_metadata_in(root, store::ITEMS_DIR)
}

/// Walk `.joy/jobs/` and return one `ItemMeta` per job file.
pub fn list_job_metadata(root: &Path) -> Result<Vec<ItemMeta>, JoyError> {
    list_metadata_in(root, store::JOBS_DIR)
}

fn list_metadata_in(root: &Path, sub: &str) -> Result<Vec<ItemMeta>, JoyError> {
    let items_dir = store::joy_dir(root).join(sub);
    if !items_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&items_dir).map_err(|e| JoyError::ReadFile {
        path: items_dir.clone(),
        source: e,
    })? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(id) = id_from_filename(name) else {
            continue;
        };
        let bytes = std::fs::read(&path).map_err(|e| JoyError::ReadFile {
            path: path.clone(),
            source: e,
        })?;
        let (encrypted_zone, plaintext_crypt_zone) = if crate::crypt::looks_like_blob(&bytes) {
            (parse_blob_zone(&bytes), None)
        } else {
            (None, parse_plaintext_crypt_zone(&bytes))
        };
        out.push(ItemMeta {
            id,
            path,
            encrypted_zone,
            plaintext_crypt_zone,
        });
    }
    Ok(out)
}

fn id_from_filename(name: &str) -> Option<String> {
    // Item filenames look like `<ID>-<title-slug>.yaml`. The ID is
    // either ACRONYM-XXXX or ACRONYM-XXXX-YY (per ADR-027). Strip
    // the `.yaml` suffix and split on the last segment that doesn't
    // match the ID shape.
    let stem = name.strip_suffix(".yaml")?;
    let parts: Vec<&str> = stem.split('-').collect();
    // Job filenames: ACRONYM-JOB-XXXX[-YY]-slug (JOY-01FE-37).
    if parts.len() >= 3
        && parts[1] == "JOB"
        && parts[2].chars().all(|c| c.is_ascii_hexdigit())
        && parts[2].len() == 4
    {
        let id_end = if parts.len() >= 4
            && parts[3].chars().all(|c| c.is_ascii_hexdigit())
            && parts[3].len() == 2
        {
            4
        } else {
            3
        };
        return Some(parts[..id_end].join("-"));
    }
    if parts.len() >= 2 && parts[1].chars().all(|c| c.is_ascii_hexdigit()) && parts[1].len() == 4 {
        // ACRONYM-XXXX[-YY]-...
        let id_end = if parts.len() >= 3
            && parts[2].chars().all(|c| c.is_ascii_hexdigit())
            && parts[2].len() == 2
        {
            3
        } else {
            2
        };
        Some(parts[..id_end].join("-"))
    } else {
        None
    }
}

fn parse_blob_zone(bytes: &[u8]) -> Option<String> {
    // Layout: 8-byte magic + 1 version + 1 zone-len + zone bytes + ...
    if bytes.len() < 10 {
        return None;
    }
    let zone_len = bytes[9] as usize;
    if bytes.len() < 10 + zone_len {
        return None;
    }
    std::str::from_utf8(&bytes[10..10 + zone_len])
        .ok()
        .map(str::to_string)
}

fn parse_plaintext_crypt_zone(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("crypt_zone:") {
            let value = rest.trim().trim_matches(|c: char| c == '"' || c == '\'');
            if value.is_empty() || value == "null" || value == "~" {
                return None;
            }
            return Some(value.to_string());
        }
    }
    None
}

/// Atomic write: temp file in the same directory, fsync, rename.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), JoyError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| JoyError::CreateDir {
        path: parent.to_path_buf(),
        source: e,
    })?;
    let tmp = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("item"),
        std::process::id()
    ));
    std::fs::write(&tmp, bytes).map_err(|e| JoyError::WriteFile {
        path: tmp.clone(),
        source: e,
    })?;
    std::fs::rename(&tmp, path).map_err(|e| JoyError::WriteFile {
        path: path.to_path_buf(),
        source: e,
    })?;
    Ok(())
}

/// Generate the next item ID by scanning existing files.
/// Returns "ACRONYM-0001" for the first item, increments the highest found.
/// All items share one number space regardless of type.
///
/// Legacy format (existing items): ACRONYM-XXXX (4 hex digits)
/// New format (ADR-027): ACRONYM-XXXX-YY (4 hex digits + 2 hex title hash)
pub fn next_id(root: &Path, acronym: &str, title: &str) -> Result<String, JoyError> {
    let prefix = acronym;

    let items_dir = store::joy_dir(root).join(store::ITEMS_DIR);
    if !items_dir.is_dir() {
        let suffix = title_hash_suffix(title);
        return Ok(format!("{prefix}-0001-{suffix}"));
    }

    let mut max_num: u16 = 0;

    let entries = std::fs::read_dir(&items_dir).map_err(|e| JoyError::ReadFile {
        path: items_dir.clone(),
        source: e,
    })?;

    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(hex_part) = name.strip_prefix(&format!("{prefix}-")) {
            if let Some(hex_str) = hex_part.get(..4) {
                if let Ok(num) = u16::from_str_radix(hex_str, 16) {
                    max_num = max_num.max(num);
                }
            }
        }
    }

    let next = max_num.checked_add(1).ok_or_else(|| {
        JoyError::Other(format!("{prefix} ID space exhausted (max {prefix}-FFFF)"))
    })?;
    let suffix = title_hash_suffix(title);
    Ok(format!("{prefix}-{next:04X}-{suffix}"))
}

/// Generate the next job item ID by scanning `.joy/jobs/`. Jobs count
/// in their own number space: `<ACRONYM>-JOB-0001-YY`, same collision
/// hash as items (ADR-027). JOY-01FE-37.
pub fn next_job_id(root: &Path, acronym: &str, title: &str) -> Result<String, JoyError> {
    let prefix = format!("{acronym}-JOB");
    let jobs_dir = store::joy_dir(root).join(store::JOBS_DIR);
    let suffix = title_hash_suffix(title);
    if !jobs_dir.is_dir() {
        return Ok(format!("{prefix}-0001-{suffix}"));
    }
    let mut max_num: u16 = 0;
    let entries = std::fs::read_dir(&jobs_dir).map_err(|e| JoyError::ReadFile {
        path: jobs_dir.clone(),
        source: e,
    })?;
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(hex_part) = name.strip_prefix(&format!("{prefix}-")) {
            if let Some(hex_str) = hex_part.get(..4) {
                if let Ok(num) = u16::from_str_radix(hex_str, 16) {
                    max_num = max_num.max(num);
                }
            }
        }
    }
    let next = max_num.checked_add(1).ok_or_else(|| {
        JoyError::Other(format!("{prefix} ID space exhausted (max {prefix}-FFFF)"))
    })?;
    Ok(format!("{prefix}-{next:04X}-{suffix}"))
}

/// Generate 2 hex digits from the title for collision-safe IDs (ADR-027).
pub fn title_hash_suffix(title: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(title.as_bytes());
    let hash = hasher.finalize();
    format!("{:02X}", hash[0])
}

/// Find the file path for an item by its ID.
/// Accepts both full IDs (JOY-0042-A3) and short-form (JOY-0042).
/// Short-form returns an error if ambiguous (multiple matches).
pub fn find_item_file(root: &Path, id: &str) -> Result<std::path::PathBuf, JoyError> {
    // The -JOB- segment routes to .joy/jobs/; everything else lives in
    // .joy/items/. Never scans both. JOY-01FE-37.
    let sub = if is_job_id(id) {
        store::JOBS_DIR
    } else {
        store::ITEMS_DIR
    };
    let items_dir = store::joy_dir(root).join(sub);
    if is_job_id(id) && !items_dir.is_dir() {
        return Err(JoyError::ItemNotFound(id.to_string()));
    }

    // Normalize: uppercase the ID for matching
    let id_upper = id.to_uppercase();

    let entries: Vec<_> = std::fs::read_dir(&items_dir)
        .map_err(|e| JoyError::ReadFile {
            path: items_dir.clone(),
            source: e,
        })?
        .filter_map(|e| e.ok())
        .collect();

    // First try exact match (full ID)
    let exact_prefix = format!("{}-", id_upper);
    for entry in &entries {
        let name = entry.file_name();
        let name_upper = name.to_string_lossy().to_uppercase();
        if name_upper.starts_with(&exact_prefix) {
            return Ok(entry.path());
        }
    }

    // Then try short-form match (prefix without suffix)
    // JOY-0042 matches JOY-0042-A3-some-title.yaml
    let short_prefix = format!("{}-", id_upper);
    let mut matches: Vec<std::path::PathBuf> = Vec::new();
    for entry in &entries {
        let name = entry.file_name();
        let name_upper = name.to_string_lossy().to_uppercase();
        if name_upper.starts_with(&short_prefix) {
            matches.push(entry.path());
        }
    }

    match matches.len() {
        0 => Err(JoyError::ItemNotFound(id.to_string())),
        1 => Ok(matches.into_iter().next().unwrap()),
        _ => {
            // Extract full IDs from filenames for the error message
            let ids: Vec<String> = matches
                .iter()
                .filter_map(|p| {
                    let name = p.file_name()?.to_string_lossy().to_string();
                    extract_full_id(&name)
                })
                .collect();
            Err(JoyError::Other(format!("ambiguous ID: {}", ids.join(", "))))
        }
    }
}

/// Extract the full item ID from a filename.
/// "JOY-0042-A3-fix-login.yaml" -> "JOY-0042-A3"
/// "JOY-0042-fix-login.yaml" -> "JOY-0042" (legacy)
fn extract_full_id(filename: &str) -> Option<String> {
    // Strip .yaml extension
    let name = filename
        .strip_suffix(".yaml")
        .or_else(|| filename.strip_suffix(".yml"))?;
    // Find acronym-XXXX pattern
    let parts: Vec<&str> = name.splitn(2, '-').collect();
    if parts.len() < 2 {
        return None;
    }
    let acronym = parts[0];
    let rest = parts[1];

    // Job format: JOB-XXXX[-YY]-slug (JOY-01FE-37)
    if let Some(job_rest) = rest.strip_prefix("JOB-") {
        let hex4 = job_rest.get(..4)?;
        if u16::from_str_radix(hex4, 16).is_err() {
            return None;
        }
        if job_rest.len() >= 7 && job_rest.as_bytes()[4] == b'-' {
            let maybe_suffix = &job_rest[5..7];
            if u8::from_str_radix(maybe_suffix, 16).is_ok()
                && (job_rest.len() == 7 || job_rest.as_bytes()[7] == b'-')
            {
                return Some(format!("{acronym}-JOB-{hex4}-{maybe_suffix}").to_uppercase());
            }
        }
        return Some(format!("{acronym}-JOB-{hex4}").to_uppercase());
    }

    // Check if it's new format: XXXX-YY-slug or legacy: XXXX-slug
    if rest.len() >= 7 && rest.as_bytes()[4] == b'-' {
        // Could be XXXX-YY-slug (new) or XXXX-slug with short slug
        let hex4 = &rest[..4];
        let maybe_suffix = &rest[5..7];
        if u16::from_str_radix(hex4, 16).is_ok()
            && maybe_suffix.len() == 2
            && u8::from_str_radix(maybe_suffix, 16).is_ok()
            && (rest.len() == 7 || rest.as_bytes()[7] == b'-')
        {
            return Some(format!("{}-{}-{}", acronym, hex4, maybe_suffix).to_uppercase());
        }
    }

    // Legacy format: XXXX-slug
    let hex4 = &rest[..4.min(rest.len())];
    if hex4.len() == 4 && u16::from_str_radix(hex4, 16).is_ok() {
        return Some(format!("{}-{}", acronym, hex4).to_uppercase());
    }

    None
}

/// Load a single item by ID.
///
/// Goes through `load_items` so that short-form ID references in
/// `parent` and `deps` are normalized to full form before the caller
/// sees them. This guarantees that any subsequent `update_item` call
/// persists the normalized form.
pub fn load_item(root: &Path, id: &str) -> Result<Item, JoyError> {
    let path = find_item_file(root, id)?;
    let target_id: String = store::read_yaml::<Item>(&path)?.id;
    let items = if is_job_id(id) {
        load_jobs(root)?
    } else {
        load_items(root)?
    };
    items
        .into_iter()
        .find(|i| i.id == target_id)
        .ok_or(JoyError::ItemNotFound(target_id))
}

/// Delete an item by ID. Returns the deleted item.
pub fn delete_item(root: &Path, id: &str) -> Result<Item, JoyError> {
    let path = find_item_file(root, id)?;
    let item: Item = store::read_yaml(&path)?;
    let rel = path
        .strip_prefix(root)
        .unwrap_or(&path)
        .to_string_lossy()
        .to_string();
    std::fs::remove_file(&path).map_err(|e| JoyError::WriteFile { path, source: e })?;
    crate::git_ops::auto_git_add(root, &[&rel]);
    Ok(item)
}

/// Remove references to a deleted item from other items' deps and parent fields.
/// `updated_by` is recorded on each touched item so the audit trail names
/// the actor who triggered the dereference.
pub fn remove_references(
    root: &Path,
    deleted_id: &str,
    updated_by: &str,
) -> Result<Vec<String>, JoyError> {
    let items = load_items(root)?;
    let mut updated = Vec::new();
    for mut item in items {
        let mut changed = false;
        if item.deps.contains(&deleted_id.to_string()) {
            item.deps.retain(|d| d != deleted_id);
            changed = true;
        }
        if item.parent.as_deref() == Some(deleted_id) {
            item.parent = None;
            changed = true;
        }
        if changed {
            touch_for_attribute_change(&mut item, updated_by);
            update_item(root, &item)?;
            updated.push(item.id.clone());
        }
    }
    Ok(updated)
}

/// Check if adding a dependency would create a cycle.
/// Returns the cycle path if one exists.
pub fn detect_cycle(
    root: &Path,
    item_id: &str,
    new_dep_id: &str,
) -> Result<Option<Vec<String>>, JoyError> {
    let items = load_items(root)?;
    let mut visited = vec![item_id.to_string()];
    if find_cycle(&items, new_dep_id, &mut visited) {
        visited.push(new_dep_id.to_string());
        Ok(Some(visited))
    } else {
        Ok(None)
    }
}

fn find_cycle(items: &[Item], current: &str, visited: &mut Vec<String>) -> bool {
    if visited.contains(&current.to_string()) {
        return true;
    }
    if let Some(item) = items.iter().find(|i| i.id == current) {
        visited.push(current.to_string());
        for dep in &item.deps {
            if find_cycle(items, dep, visited) {
                return true;
            }
        }
        visited.pop();
    }
    false
}

/// Update an item in place (overwrites its file).
pub fn update_item(root: &Path, item: &Item) -> Result<(), JoyError> {
    let old_path = find_item_file(root, &item.id)?;
    // Write new file first to avoid data loss if write fails
    save_item(root, item)?;
    // Remove old file if the filename changed (title may have changed)
    let new_path = dir_for_type(root, &item.item_type).join(item_filename(&item.id, &item.title));
    if old_path != new_path {
        let _ = std::fs::remove_file(&old_path);
        let old_rel = old_path
            .strip_prefix(root)
            .unwrap_or(&old_path)
            .to_string_lossy()
            .to_string();
        crate::git_ops::auto_git_add(root, &[&old_rel]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::item::{ItemType, Priority};
    use tempfile::tempdir;

    fn setup_project(dir: &Path) {
        let joy_dir = dir.join(".joy");
        std::fs::create_dir_all(joy_dir.join("items")).unwrap();
    }

    #[test]
    fn next_id_first_item() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        let id = next_id(dir.path(), "JOY", "Test item").unwrap();
        assert!(id.starts_with("JOY-0001-"), "got: {id}");
        assert_eq!(id.len(), 11); // JOY-0001-XX
    }

    #[test]
    fn next_id_increments() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let item = Item::new(
            "JOY-0001".into(),
            "First".into(),
            ItemType::Task,
            Priority::Low,
            vec![],
        );
        save_item(dir.path(), &item).unwrap();

        let id = next_id(dir.path(), "JOY", "Second item").unwrap();
        assert!(id.starts_with("JOY-0002-"), "got: {id}");
    }

    #[test]
    fn next_id_skips_gaps() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let item1 = Item::new(
            "JOY-0001".into(),
            "First".into(),
            ItemType::Task,
            Priority::Low,
            vec![],
        );
        save_item(dir.path(), &item1).unwrap();

        let item3 = Item::new(
            "JOY-0003".into(),
            "Third".into(),
            ItemType::Task,
            Priority::Low,
            vec![],
        );
        save_item(dir.path(), &item3).unwrap();

        let id = next_id(dir.path(), "JOY", "Fourth item").unwrap();
        assert!(id.starts_with("JOY-0004-"), "got: {id}");
    }

    #[test]
    fn next_id_same_title_same_suffix() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        let id1 = next_id(dir.path(), "JOY", "Same title").unwrap();
        let suffix1 = &id1[9..];
        let id2_suffix = title_hash_suffix("Same title");
        assert_eq!(suffix1, id2_suffix);
    }

    #[test]
    fn next_id_different_titles_different_suffixes() {
        let suffix_a = title_hash_suffix("Fix login bug");
        let suffix_b = title_hash_suffix("Add roadmap feature");
        // Not guaranteed different, but astronomically unlikely to be equal
        // for these specific strings. If this test fails, the hash function
        // has a collision on these inputs (1:256 chance).
        assert_ne!(suffix_a, suffix_b);
    }

    #[test]
    fn next_id_increments_past_new_format() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        // Save an item with new format ID
        let item = Item::new(
            "JOY-0005-A3".into(),
            "New format".into(),
            ItemType::Task,
            Priority::Low,
            vec![],
        );
        save_item(dir.path(), &item).unwrap();

        let id = next_id(dir.path(), "JOY", "Next item").unwrap();
        assert!(id.starts_with("JOY-0006-"), "got: {id}");
    }

    #[test]
    fn load_items_empty() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        let items = load_items(dir.path()).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn save_and_load_item() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let item = Item::new(
            "JOY-0001".into(),
            "Test item".into(),
            ItemType::Story,
            Priority::High,
            vec![],
        );
        save_item(dir.path(), &item).unwrap();

        let items = load_items(dir.path()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "JOY-0001");
        assert_eq!(items[0].title, "Test item");
    }

    #[test]
    fn load_items_sorted() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let item2 = Item::new(
            "JOY-0002".into(),
            "Second".into(),
            ItemType::Task,
            Priority::Low,
            vec![],
        );
        save_item(dir.path(), &item2).unwrap();

        let item1 = Item::new(
            "JOY-0001".into(),
            "First".into(),
            ItemType::Task,
            Priority::Low,
            vec![],
        );
        save_item(dir.path(), &item1).unwrap();

        let items = load_items(dir.path()).unwrap();
        assert_eq!(items[0].id, "JOY-0001");
        assert_eq!(items[1].id, "JOY-0002");
    }

    #[test]
    fn short_form_extracts_prefix_for_suffixed_id() {
        assert_eq!(short_form("JOY-0042-A3"), Some("JOY-0042"));
        assert_eq!(short_form("TST-00FF-12"), Some("TST-00FF"));
    }

    #[test]
    fn short_form_returns_none_for_legacy_id() {
        assert_eq!(short_form("JOY-0042"), None);
        assert_eq!(short_form("JOY-MS-01"), None);
    }

    #[test]
    fn short_form_returns_none_for_non_hex_suffix() {
        assert_eq!(short_form("JOY-0042-XX"), None);
        assert_eq!(short_form("JOY-0042-AAA"), None);
    }

    #[test]
    fn normalize_rewrites_short_form_parent() {
        let mut parent = Item::new(
            "JOY-0042-A3".into(),
            "P".into(),
            ItemType::Epic,
            Priority::Medium,
            vec![],
        );
        parent.parent = None;
        let mut child = Item::new(
            "JOY-0043-B1".into(),
            "C".into(),
            ItemType::Task,
            Priority::Medium,
            vec![],
        );
        child.parent = Some("JOY-0042".into());
        let mut items = vec![parent, child];
        normalize_id_refs(&mut items);
        assert_eq!(items[1].parent.as_deref(), Some("JOY-0042-A3"));
    }

    #[test]
    fn normalize_rewrites_short_form_deps() {
        let dep = Item::new(
            "JOY-0042-A3".into(),
            "D".into(),
            ItemType::Task,
            Priority::Medium,
            vec![],
        );
        let mut consumer = Item::new(
            "JOY-0043-B1".into(),
            "C".into(),
            ItemType::Task,
            Priority::Medium,
            vec![],
        );
        consumer.deps = vec!["JOY-0042".into()];
        let mut items = vec![dep, consumer];
        normalize_id_refs(&mut items);
        assert_eq!(items[1].deps, vec!["JOY-0042-A3".to_string()]);
    }

    #[test]
    fn normalize_leaves_full_form_unchanged() {
        let parent = Item::new(
            "JOY-0042-A3".into(),
            "P".into(),
            ItemType::Epic,
            Priority::Medium,
            vec![],
        );
        let mut child = Item::new(
            "JOY-0043-B1".into(),
            "C".into(),
            ItemType::Task,
            Priority::Medium,
            vec![],
        );
        child.parent = Some("JOY-0042-A3".into());
        let mut items = vec![parent, child];
        normalize_id_refs(&mut items);
        assert_eq!(items[1].parent.as_deref(), Some("JOY-0042-A3"));
    }

    #[test]
    fn normalize_leaves_unknown_refs_unchanged() {
        let mut child = Item::new(
            "JOY-0043-B1".into(),
            "C".into(),
            ItemType::Task,
            Priority::Medium,
            vec![],
        );
        child.parent = Some("JOY-9999".into());
        child.deps = vec!["JOY-8888".into()];
        let mut items = vec![child];
        normalize_id_refs(&mut items);
        assert_eq!(items[0].parent.as_deref(), Some("JOY-9999"));
        assert_eq!(items[0].deps, vec!["JOY-8888".to_string()]);
    }

    #[test]
    fn normalize_leaves_ambiguous_short_forms_unchanged() {
        let a = Item::new(
            "JOY-0042-A3".into(),
            "A".into(),
            ItemType::Task,
            Priority::Medium,
            vec![],
        );
        let b = Item::new(
            "JOY-0042-B1".into(),
            "B".into(),
            ItemType::Task,
            Priority::Medium,
            vec![],
        );
        let mut child = Item::new(
            "JOY-0043-CC".into(),
            "C".into(),
            ItemType::Task,
            Priority::Medium,
            vec![],
        );
        child.parent = Some("JOY-0042".into());
        let mut items = vec![a, b, child];
        normalize_id_refs(&mut items);
        assert_eq!(items[2].parent.as_deref(), Some("JOY-0042"));
    }

    #[test]
    fn milestone_short_form_extracts_prefix() {
        assert_eq!(milestone_short_form("JOY-MS-01-A1"), Some("JOY-MS-01"));
        assert_eq!(milestone_short_form("TST-MS-FF-12"), Some("TST-MS-FF"));
    }

    #[test]
    fn milestone_short_form_returns_none_for_legacy_or_item() {
        assert_eq!(milestone_short_form("JOY-MS-01"), None);
        assert_eq!(milestone_short_form("JOY-0042-A3"), None);
    }

    #[test]
    fn normalize_milestone_rewrites_short_form() {
        let mut item = Item::new(
            "JOY-0001-AA".into(),
            "X".into(),
            ItemType::Task,
            Priority::Medium,
            vec![],
        );
        item.milestone = Some("JOY-MS-01".into());
        let mut items = vec![item];
        normalize_milestone_refs(&mut items, &["JOY-MS-01-A1".to_string()]);
        assert_eq!(items[0].milestone.as_deref(), Some("JOY-MS-01-A1"));
    }

    #[test]
    fn normalize_milestone_leaves_unknown_unchanged() {
        let mut item = Item::new(
            "JOY-0001-AA".into(),
            "X".into(),
            ItemType::Task,
            Priority::Medium,
            vec![],
        );
        item.milestone = Some("JOY-MS-99".into());
        let mut items = vec![item];
        normalize_milestone_refs(&mut items, &["JOY-MS-01-A1".to_string()]);
        assert_eq!(items[0].milestone.as_deref(), Some("JOY-MS-99"));
    }

    #[test]
    fn normalize_milestone_leaves_full_form_unchanged() {
        let mut item = Item::new(
            "JOY-0001-AA".into(),
            "X".into(),
            ItemType::Task,
            Priority::Medium,
            vec![],
        );
        item.milestone = Some("JOY-MS-01-A1".into());
        let mut items = vec![item];
        normalize_milestone_refs(&mut items, &["JOY-MS-01-A1".to_string()]);
        assert_eq!(items[0].milestone.as_deref(), Some("JOY-MS-01-A1"));
    }

    #[test]
    fn normalize_handles_legacy_parent_referenced_by_full_id() {
        let parent = Item::new(
            "JOY-0042".into(),
            "P".into(),
            ItemType::Epic,
            Priority::Medium,
            vec![],
        );
        let mut child = Item::new(
            "JOY-0043-B1".into(),
            "C".into(),
            ItemType::Task,
            Priority::Medium,
            vec![],
        );
        child.parent = Some("JOY-0042".into());
        let mut items = vec![parent, child];
        normalize_id_refs(&mut items);
        assert_eq!(items[1].parent.as_deref(), Some("JOY-0042"));
    }
}
