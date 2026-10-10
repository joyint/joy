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

/// Apply a `joy edit --scope` spec to a job's CURRENT scope. The ONE
/// definition shared by the CLI, the desktop shell and the platform (ADR
/// JAPP-011A-9F: no divergent second implementation, e.g. the TS copy in
/// the web wiring or the desktop shell's own copy). A plain CSV replaces
/// the scope; `+ID`/`-ID` entries add/remove; mixing the two forms is
/// rejected; an addition must resolve to an existing non-job item; the
/// result is never empty.
pub fn apply_scope_spec(
    root: &Path,
    current: &[String],
    spec: &str,
) -> Result<Vec<String>, JoyError> {
    let entries: Vec<&str> = spec
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if entries.is_empty() {
        return Err(JoyError::GuardDenied(
            "a job needs at least one scope item".into(),
        ));
    }
    let delta = entries
        .iter()
        .any(|e| e.starts_with('+') || e.starts_with('-'));
    let mut scope: Vec<String>;
    if delta {
        if !entries
            .iter()
            .all(|e| e.starts_with('+') || e.starts_with('-'))
        {
            return Err(JoyError::GuardDenied(
                "--scope cannot mix +/- entries with plain IDs; use one form".into(),
            ));
        }
        scope = current.to_vec();
        for entry in &entries {
            let (op, sid) = entry.split_at(1);
            let sid = sid.trim();
            if op == "+" {
                let full = resolve_scope_item(root, sid)?;
                if !scope.contains(&full) {
                    scope.push(full);
                }
            } else {
                // Normalize a short form when it still resolves; a stale ID
                // that no longer loads is matched verbatim.
                let full = load_item(root, sid)
                    .map(|i| i.id)
                    .unwrap_or_else(|_| sid.to_string());
                let before = scope.len();
                scope.retain(|s| s != &full && s != sid);
                if scope.len() == before {
                    return Err(JoyError::GuardDenied(format!(
                        "{sid} is not in the scope of this job"
                    )));
                }
            }
        }
    } else {
        scope = Vec::new();
        for sid in &entries {
            let full = resolve_scope_item(root, sid)?;
            if !scope.contains(&full) {
                scope.push(full);
            }
        }
    }
    if scope.is_empty() {
        return Err(JoyError::GuardDenied(
            "a job needs at least one scope item".into(),
        ));
    }
    Ok(scope)
}

/// Validate one scope addition: must resolve to an existing NON-job item;
/// returns the full (normalized) item id.
pub fn resolve_scope_item(root: &Path, sid: &str) -> Result<String, JoyError> {
    if is_job_id(sid) {
        return Err(JoyError::GuardDenied(
            "a job cannot scope another job; use deps for job ordering".into(),
        ));
    }
    let scope_item = load_item(root, sid)
        .map_err(|_| JoyError::GuardDenied(format!("scope item {sid} is not a valid item ID.")))?;
    if scope_item.item_type == ItemType::Job {
        return Err(JoyError::GuardDenied(
            "a job cannot scope another job; use deps for job ordering".into(),
        ));
    }
    Ok(scope_item.id)
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

/// Read one item file: YAML (decrypting a JOYCRYPT blob inside
/// `store::read_yaml`), then the item schema migrations
/// (`migrations::item_yaml`), then the strict typed model. Every item
/// read goes through here or through [`item_from_bytes`], so the model
/// never needs a tolerant field; the migrated form persists when the
/// item is next saved.
pub fn read_item_file(path: &Path) -> Result<Item, JoyError> {
    item_from_bytes(path, read_item_bytes(path)?)
}

/// [`read_item_file`] for a caller that already holds the file's bytes.
fn item_from_bytes(path: &Path, bytes: Vec<u8>) -> Result<Item, JoyError> {
    let value: serde_yaml_ng::Value = store::yaml_from_bytes(path, bytes)?;
    let (value, _migrated) = crate::migrations::item_yaml::apply(value);
    serde_yaml_ng::from_value(value).map_err(|e| JoyError::YamlParse {
        path: path.to_path_buf(),
        source: e,
    })
}

/// The one place an item or job file is read from disk. A read costs
/// the same handful of file accesses whatever the file holds, and on a
/// slow filesystem those accesses are the whole cost of a command, so
/// every pass reads a file once and hands the bytes on (JOY-02B7-B7).
fn read_item_bytes(path: &Path) -> Result<Vec<u8>, JoyError> {
    #[cfg(test)]
    read_count::bump();
    std::fs::read(path).map_err(|e| JoyError::ReadFile {
        path: path.to_path_buf(),
        source: e,
    })
}

/// How many item files this thread has read, so a test can say that a
/// pass reads each file once.
#[cfg(test)]
pub(crate) mod read_count {
    use std::cell::Cell;

    thread_local! {
        static READS: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) fn bump() {
        READS.with(|reads| reads.set(reads.get() + 1));
    }

    /// The reads `work` caused on this thread.
    pub(crate) fn during<T>(work: impl FnOnce() -> T) -> (T, usize) {
        let before = READS.with(Cell::get);
        let out = work();
        (out, READS.with(Cell::get) - before)
    }
}

/// Load all items from `.joy/items/`, separating decryptable ones from
/// encrypted blobs the caller has no zone-key for. Plaintext items and
/// items whose zone key is currently active are returned as `Item`;
/// items in zones without an active key are returned as
/// [`LockedItem`] placeholders. See JOY-0174-D3.
pub fn load_items_with_locked(root: &Path) -> Result<(Vec<Item>, Vec<LockedItem>), JoyError> {
    let mut files = scan_dir(root, store::ITEMS_DIR)?;
    files.sort_by(|a, b| a.meta.path.file_name().cmp(&b.meta.path.file_name()));

    let mut items: Vec<Item> = Vec::new();
    let mut locked: Vec<LockedItem> = Vec::new();
    for ScannedFile { meta, bytes } in files {
        if let Some(zone) = meta.encrypted_zone.as_deref() {
            if crate::crypt::active_zone_key(zone).is_none() {
                locked.push(LockedItem {
                    id: meta.id,
                    zone: zone.to_string(),
                });
                continue;
            }
        }
        let item = item_from_bytes(&meta.path, bytes)?;
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
    let mut files = scan_dir(root, store::JOBS_DIR)?;
    files.sort_by(|a, b| a.meta.path.file_name().cmp(&b.meta.path.file_name()));
    let mut jobs: Vec<Item> = Vec::new();
    for ScannedFile { meta, bytes } in files {
        if let Some(zone) = meta.encrypted_zone.as_deref() {
            if crate::crypt::active_zone_key(zone).is_none() {
                continue;
            }
        }
        let item = item_from_bytes(&meta.path, bytes)?;
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

/// Record a mutation on an item: `updated` / `updated_by` for sort
/// recency, nothing else. The audit history is NOT stored here — the
/// event log is the one record of who changed what when (decision
/// JOY-0175-9B), and every display derives the "Updated" trail from it
/// at lookup time.
pub fn touch_for_attribute_change(item: &mut Item, by: &str) {
    let now = chrono::Utc::now();
    item.updated = now;
    item.updated_by = Some(by.into());
}

/// `touch_for_attribute_change`, but only when the item actually differs
/// from `before` (its state prior to the edit). Returns whether it
/// touched. Re-sending an item's current values is not a change: without
/// this guard every replayed edit bumps `updated`, swaps `updated_by` to
/// an actor who changed nothing, and appends a history entry (a looping
/// client once flooded a job with 600+ such entries). Callers snapshot
/// the item right after loading it, mutate, then let this decide:
///
/// ```ignore
/// let before = item.clone();
/// // ... apply the requested edits ...
/// if touch_if_changed(&mut item, &before, &user) {
///     update_item(root, &item)?;
/// }
/// ```
pub fn touch_if_changed(item: &mut Item, before: &Item, by: &str) -> bool {
    if item == before {
        return false;
    }
    touch_for_attribute_change(item, by);
    true
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
    // A job that says `until` is something a joy before reads wrong: it
    // shows the job without its end and drops the end at its next write
    // (JOY-02CA-EE). The project says so first, in the same write, so an
    // older joy meets the number before it meets the field.
    let says_until = item
        .job
        .as_ref()
        .and_then(|job| job.window.as_ref())
        .is_some_and(|window| window.until.is_some());
    if says_until {
        store::raise_format(root, store::FORMAT_JOB_UNTIL)?;
    }
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
            joy_crypt::zone::encrypt_blob(zone, &zone_key, yaml.as_bytes())
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
    Ok(scan_dir(root, sub)?
        .into_iter()
        .map(|file| file.meta)
        .collect())
}

/// One item file as the directory walk met it: what the walk learned
/// about it, and the bytes it read to learn that. A caller that goes on
/// to parse the item takes the bytes instead of reading the file again.
struct ScannedFile {
    meta: ItemMeta,
    bytes: Vec<u8>,
}

fn scan_dir(root: &Path, sub: &str) -> Result<Vec<ScannedFile>, JoyError> {
    let items_dir = store::joy_dir(root).join(sub);
    let entries = match std::fs::read_dir(&items_dir) {
        Ok(entries) => entries,
        // No such directory is a project without items of this kind.
        Err(_) if !items_dir.is_dir() => return Ok(Vec::new()),
        Err(e) => {
            return Err(JoyError::ReadFile {
                path: items_dir.clone(),
                source: e,
            })
        }
    };
    let mut out = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !is_regular_file(&entry, &path) {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(id) = id_from_filename(name) else {
            continue;
        };
        let bytes = read_item_bytes(&path)?;
        let (encrypted_zone, plaintext_crypt_zone) = if joy_crypt::zone::looks_like_blob(&bytes) {
            (parse_blob_zone(&bytes), None)
        } else {
            (None, parse_plaintext_crypt_zone(&bytes))
        };
        out.push(ScannedFile {
            meta: ItemMeta {
                id,
                path,
                encrypted_zone,
                plaintext_crypt_zone,
            },
            bytes,
        });
    }
    Ok(out)
}

/// Whether a directory entry is a file, the way `Path::is_file` answers
/// (a link to a file counts). The listing already carries the type on
/// the filesystems joy meets, so only a link costs a further access.
fn is_regular_file(entry: &std::fs::DirEntry, path: &Path) -> bool {
    match entry.file_type() {
        Ok(kind) if kind.is_file() => true,
        Ok(kind) if kind.is_dir() => false,
        _ => path.is_file(),
    }
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
    let (dir, names) = item_dir_names(root, id)?;
    Ok(dir.join(match_item_name(&names, id)?))
}

/// The directory an ID routes to, and the names in it: one listing that
/// a lookup and the reference resolution after it can both use.
fn item_dir_names(
    root: &Path,
    id: &str,
) -> Result<(std::path::PathBuf, Vec<std::ffi::OsString>), JoyError> {
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

    let names = std::fs::read_dir(&items_dir)
        .map_err(|e| JoyError::ReadFile {
            path: items_dir.clone(),
            source: e,
        })?
        .filter_map(|e| e.ok())
        .map(|entry| entry.file_name())
        .collect();
    Ok((items_dir, names))
}

/// The name in `names` that carries `id`, full or short form.
fn match_item_name<'n>(
    names: &'n [std::ffi::OsString],
    id: &str,
) -> Result<&'n std::ffi::OsStr, JoyError> {
    // Normalize: uppercase the ID for matching
    let id_upper = id.to_uppercase();

    // First try exact match (full ID)
    let exact_prefix = format!("{}-", id_upper);
    for name in names {
        let name_upper = name.to_string_lossy().to_uppercase();
        if name_upper.starts_with(&exact_prefix) {
            return Ok(name);
        }
    }

    // Then try short-form match (prefix without suffix)
    // JOY-0042 matches JOY-0042-A3-some-title.yaml
    let short_prefix = format!("{}-", id_upper);
    let mut matches: Vec<&std::ffi::OsStr> = Vec::new();
    for name in names {
        let name_upper = name.to_string_lossy().to_uppercase();
        if name_upper.starts_with(&short_prefix) {
            matches.push(name);
        }
    }

    match matches.len() {
        0 => Err(JoyError::ItemNotFound(id.to_string())),
        1 => Ok(matches[0]),
        _ => {
            // Extract full IDs from filenames for the error message
            let ids: Vec<String> = matches
                .iter()
                .filter_map(|name| extract_full_id(&name.to_string_lossy()))
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
/// Short-form ID references in `parent`, `deps` and `milestone` are
/// normalized to full form before the caller sees them, as
/// [`load_items`] does for the whole set. This guarantees that any
/// subsequent `update_item` call persists the normalized form.
///
/// It reads the item's own file and nothing else, unless the item
/// still carries a short-form reference: then the files that reference
/// could mean are read to learn their IDs (JOY-02B8-78). Loading the
/// whole set here made every command on one item cost a pass over all
/// of them.
pub fn load_item(root: &Path, id: &str) -> Result<Item, JoyError> {
    let (dir, names) = item_dir_names(root, id)?;
    let path = dir.join(match_item_name(&names, id)?);
    let mut item = read_item_file(&path)?;
    // Jobs carry no references that are normalized (see `load_jobs`).
    if !is_job_id(id) {
        resolve_short_id_refs(&mut item, &dir, &names);
        resolve_short_milestone_ref(root, &mut item);
    }
    Ok(item)
}

/// [`normalize_id_refs`] for one item: rewrite a short-form `parent` or
/// dependency to the full ID it stands for, and leave it alone when no
/// item or more than one answers to it.
///
/// The IDs come from the files a reference could mean, not from their
/// names: a legacy `ACRONYM-XXXX` file whose title starts with two hex
/// characters reads like a suffixed ID by name alone. A file that cannot
/// be read (a zone without a key) does not answer, as it is absent from
/// the set [`load_items`] normalizes over.
fn resolve_short_id_refs(item: &mut Item, dir: &Path, names: &[std::ffi::OsString]) {
    let resolve = |reference: &str| -> Option<String> {
        // Already a full ID: nothing it could be short for.
        if short_form(reference).is_some() {
            return None;
        }
        let prefix = format!("{reference}-");
        let mut full: Option<String> = None;
        for name in names {
            if !name.to_string_lossy().starts_with(&prefix) {
                continue;
            }
            let Ok(candidate) = read_item_file(&dir.join(name)) else {
                continue;
            };
            if short_form(&candidate.id) == Some(reference) {
                if full.is_some() {
                    return None;
                }
                full = Some(candidate.id);
            }
        }
        full
    };
    if let Some(full) = item.parent.as_deref().and_then(resolve) {
        item.parent = Some(full);
    }
    for dep in &mut item.deps {
        if let Some(full) = resolve(dep) {
            *dep = full;
        }
    }
}

/// [`normalize_milestone_refs`] for one item. The milestones are only
/// read when the item names one in a form that could be short.
fn resolve_short_milestone_ref(root: &Path, item: &mut Item) {
    let Some(milestone) = item.milestone.as_deref() else {
        return;
    };
    if milestone_short_form(milestone).is_some() {
        return;
    }
    let milestone_ids: Vec<String> = crate::milestones::load_milestones(root)
        .map(|list| list.into_iter().map(|m| m.id).collect())
        .unwrap_or_default();
    normalize_milestone_refs(std::slice::from_mut(item), &milestone_ids);
}

/// Delete an item by ID. Returns the deleted item.
pub fn delete_item(root: &Path, id: &str) -> Result<Item, JoyError> {
    let path = find_item_file(root, id)?;
    let item = read_item_file(&path)?;
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
    let mut items = load_items(root)?;
    remove_references_in(root, &mut items, deleted_id, updated_by)
}

/// [`remove_references`] over a set the caller already loaded, so a
/// command that deletes several items reads the item files once and not
/// once per deleted item (JOY-02B8-78).
///
/// `items` is kept true to the disk: the deleted item leaves it and a
/// dereferenced item is changed in it, so the next call neither writes
/// the deleted item back nor undoes an earlier dereference.
pub fn remove_references_in(
    root: &Path,
    items: &mut Vec<Item>,
    deleted_id: &str,
    updated_by: &str,
) -> Result<Vec<String>, JoyError> {
    items.retain(|item| item.id != deleted_id);
    let mut updated = Vec::new();
    for item in items.iter_mut() {
        let mut changed = false;
        if item.deps.iter().any(|d| d == deleted_id) {
            item.deps.retain(|d| d != deleted_id);
            changed = true;
        }
        if item.parent.as_deref() == Some(deleted_id) {
            item.parent = None;
            changed = true;
        }
        if changed {
            touch_for_attribute_change(item, updated_by);
            update_item(root, item)?;
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
    #[test]
    fn a_stored_history_block_sheds_on_the_items_next_save() {
        // The migration (item_yaml) drops the retired key on read; the
        // normal save persists the strict schema, so the FILE loses the
        // block exactly when the item is saved anyway — no sweep.
        let dir = tempfile::tempdir().unwrap();
        let items_dir = dir.path().join(".joy").join("items");
        std::fs::create_dir_all(&items_dir).unwrap();
        // the canonical name save_item writes back to ({ID}-{slug}.yaml)
        let file = items_dir.join(crate::model::item::item_filename("T-0001-AB", "old"));
        std::fs::write(
            &file,
            concat!(
                "id: T-0001-AB\n",
                "title: old\n",
                "type: task\n",
                "status: new\n",
                "priority: medium\n",
                "created: 2026-01-01T00:00:00Z\n",
                "updated: 2026-01-01T00:00:00Z\n",
                "history:\n",
                "- date: 2026-01-02T00:00:00Z\n",
                "  by: a@x\n",
            ),
        )
        .unwrap();

        let item = super::read_item_file(&file).unwrap();
        super::save_item(dir.path(), &item).unwrap();

        let saved = std::fs::read_dir(&items_dir)
            .unwrap()
            .flatten()
            .map(|e| std::fs::read_to_string(e.path()).unwrap())
            .collect::<String>();
        assert!(saved.contains("title: old"), "{saved}");
        assert!(!saved.contains("history"), "{saved}");
    }

    use super::*;
    use crate::model::item::{ItemType, Priority};
    use tempfile::tempdir;

    fn setup_project(dir: &Path) {
        let joy_dir = dir.join(".joy");
        std::fs::create_dir_all(joy_dir.join("items")).unwrap();
    }

    #[test]
    fn touch_if_changed_skips_no_op_edits() {
        let mut item = Item::new(
            "JOY-0001".into(),
            "Stable".into(),
            ItemType::Task,
            Priority::Low,
            vec![],
        );
        let before = item.clone();

        // replaying identical values: no history entry, no last-editor swap
        assert!(!touch_if_changed(&mut item, &before, "b@example.com"));
        assert_eq!(item, before);

        // a real change touches: updated_by moves, history grows
        item.title = "Changed".into();
        assert!(touch_if_changed(&mut item, &before, "b@example.com"));
        assert_eq!(item.updated_by.as_deref(), Some("b@example.com"));
    }

    #[test]
    fn apply_scope_spec_replaces_adds_removes_and_validates() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        let root = dir.path();
        for id in ["JOY-0001", "JOY-0002"] {
            save_item(
                root,
                &Item::new(id.into(), "T".into(), ItemType::Task, Priority::Low, vec![]),
            )
            .unwrap();
        }
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();

        // a plain list REPLACES the whole scope
        assert_eq!(
            apply_scope_spec(root, &s(&["JOY-0009"]), "JOY-0001,JOY-0002").unwrap(),
            s(&["JOY-0001", "JOY-0002"])
        );
        // +/- entries ADJUST the current scope
        assert_eq!(
            apply_scope_spec(root, &s(&["JOY-0001"]), "+JOY-0002").unwrap(),
            s(&["JOY-0001", "JOY-0002"])
        );
        assert_eq!(
            apply_scope_spec(root, &s(&["JOY-0001", "JOY-0002"]), "-JOY-0001").unwrap(),
            s(&["JOY-0002"])
        );

        // mixing plain IDs with +/- is rejected
        assert!(apply_scope_spec(root, &[], "JOY-0001,+JOY-0002").is_err());
        // adding a non-existent item is rejected
        assert!(apply_scope_spec(root, &[], "JOY-9999").is_err());
        // a job id can never be a scope item (a job cannot scope a job)
        assert!(apply_scope_spec(root, &[], "JOY-JOB-0001").is_err());
        // removing a non-member is rejected
        assert!(apply_scope_spec(root, &s(&["JOY-0001"]), "-JOY-0002").is_err());
        // the scope may never end up empty
        assert!(apply_scope_spec(root, &s(&["JOY-0001"]), "-JOY-0001").is_err());
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

    /// JOY-02B7-B7: a pass over the items reads each file once. The walk
    /// that finds the crypt zone hands its bytes to the parser, so the
    /// second read that used to follow it is gone, for the locked-aware
    /// loader and for the metadata walk alike.
    #[test]
    fn a_pass_over_the_items_reads_each_file_once() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        for n in 1..=5 {
            let item = Item::new(
                format!("JOY-000{n}-AB"),
                format!("Item number {n}"),
                ItemType::Task,
                Priority::Low,
                vec![],
            );
            save_item(dir.path(), &item).unwrap();
        }

        let (items, reads) = read_count::during(|| load_items(dir.path()).unwrap());
        assert_eq!(items.len(), 5);
        assert_eq!(reads, 5, "one read per item file");

        let (metas, reads) = read_count::during(|| list_item_metadata(dir.path()).unwrap());
        assert_eq!(metas.len(), 5);
        assert_eq!(reads, 5, "the metadata walk reads each file once too");
    }

    /// The walk skips what is not an item file, as it did when it asked
    /// `is_file()` of every entry: a directory, and a file whose name
    /// carries no item id.
    #[test]
    fn the_walk_skips_directories_and_foreign_files() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        let items_dir = dir.path().join(".joy").join("items");
        std::fs::create_dir_all(items_dir.join("JOY-0009-AB-a-directory.yaml")).unwrap();
        std::fs::write(items_dir.join("notes.txt"), "not an item").unwrap();
        let item = Item::new(
            "JOY-0001-AB".into(),
            "The only item".into(),
            ItemType::Task,
            Priority::Low,
            vec![],
        );
        save_item(dir.path(), &item).unwrap();

        let (items, reads) = read_count::during(|| load_items(dir.path()).unwrap());
        assert_eq!(items.len(), 1);
        assert_eq!(reads, 1);
    }

    /// No items directory at all is an empty project, not an error.
    #[test]
    fn a_missing_items_directory_is_an_empty_listing() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".joy")).unwrap();
        assert!(load_items(dir.path()).unwrap().is_empty());
        assert!(load_jobs(dir.path()).unwrap().is_empty());
        assert!(list_item_metadata(dir.path()).unwrap().is_empty());
    }

    fn task(id: &str, title: &str) -> Item {
        Item::new(
            id.into(),
            title.into(),
            ItemType::Task,
            Priority::Low,
            vec![],
        )
    }

    /// JOY-02B8-78: loading one item reads its file and no other. It
    /// used to load the whole set to normalize references that, in a
    /// project of full IDs, need no normalizing at all.
    #[test]
    fn loading_one_item_reads_one_file() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        save_item(dir.path(), &task("JOY-0001-AA", "The parent")).unwrap();
        save_item(dir.path(), &task("JOY-0002-BB", "A dependency")).unwrap();
        let mut item = task("JOY-0003-CC", "The one that is loaded");
        item.parent = Some("JOY-0001-AA".into());
        item.deps = vec!["JOY-0002-BB".into()];
        save_item(dir.path(), &item).unwrap();

        let (loaded, reads) = read_count::during(|| load_item(dir.path(), "JOY-0003-CC").unwrap());
        assert_eq!(loaded.id, "JOY-0003-CC");
        assert_eq!(reads, 1);

        // The short form of its own ID finds it just the same.
        let (loaded, reads) = read_count::during(|| load_item(dir.path(), "JOY-0003").unwrap());
        assert_eq!(loaded.id, "JOY-0003-CC");
        assert_eq!(reads, 1);
    }

    /// One item's short-form references come out as the whole set's
    /// normalization writes them: the unique one in full, the ambiguous
    /// and the unknown one as they stand.
    #[test]
    fn loading_one_item_resolves_short_references_like_the_whole_set() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        save_item(dir.path(), &task("JOY-0001-AA", "The parent")).unwrap();
        save_item(dir.path(), &task("JOY-0002-BB", "A dependency")).unwrap();
        // Two items share the short form JOY-0005.
        save_item(dir.path(), &task("JOY-0005-11", "One of two")).unwrap();
        save_item(dir.path(), &task("JOY-0005-22", "Two of two")).unwrap();
        let mut item = task("JOY-0003-CC", "The one that is loaded");
        item.parent = Some("JOY-0001".into());
        item.deps = vec![
            "JOY-0002".into(),
            "JOY-0005".into(),
            "JOY-0099".into(),
            "JOY-0002-BB".into(),
        ];
        save_item(dir.path(), &item).unwrap();

        let one = load_item(dir.path(), "JOY-0003-CC").unwrap();
        assert_eq!(one.parent.as_deref(), Some("JOY-0001-AA"));
        assert_eq!(
            one.deps,
            vec!["JOY-0002-BB", "JOY-0005", "JOY-0099", "JOY-0002-BB"]
        );

        let of_the_set = load_items(dir.path())
            .unwrap()
            .into_iter()
            .find(|i| i.id == "JOY-0003-CC")
            .unwrap();
        assert_eq!(one.parent, of_the_set.parent);
        assert_eq!(one.deps, of_the_set.deps);
    }

    /// A legacy ID is a full ID. Its file name can read like a suffixed
    /// one when the title starts with two hex characters, which is why
    /// the IDs are taken from the files and not from their names: the
    /// reference stays what it is.
    #[test]
    fn a_legacy_reference_is_not_taken_for_a_short_form() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        // Saved as JOY-0042-be-careful.yaml.
        save_item(dir.path(), &task("JOY-0042", "Be careful")).unwrap();
        let mut item = task("JOY-0043-CC", "Child of a legacy item");
        item.parent = Some("JOY-0042".into());
        item.deps = vec!["JOY-0042".into()];
        save_item(dir.path(), &item).unwrap();

        let one = load_item(dir.path(), "JOY-0043-CC").unwrap();
        assert_eq!(one.parent.as_deref(), Some("JOY-0042"));
        assert_eq!(one.deps, vec!["JOY-0042"]);
    }

    /// A short-form milestone reference is resolved for one item as it
    /// is for the set.
    #[test]
    fn loading_one_item_resolves_a_short_milestone_reference() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        let milestone = crate::model::Milestone::new("JOY-MS-01-A1".into(), "First".into());
        crate::milestones::save_milestone(dir.path(), &milestone).unwrap();
        let mut item = task("JOY-0001-AA", "In the milestone");
        item.milestone = Some("JOY-MS-01".into());
        save_item(dir.path(), &item).unwrap();

        let one = load_item(dir.path(), "JOY-0001-AA").unwrap();
        assert_eq!(one.milestone.as_deref(), Some("JOY-MS-01-A1"));
    }

    /// JOY-02B8-78: deleting several items works on one loaded set. The
    /// set follows the disk, so a second dereference of the same item
    /// keeps the first, and a deleted item is not written back.
    #[test]
    fn references_are_removed_from_a_set_that_follows_the_disk() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        save_item(dir.path(), &task("JOY-0001-AA", "First to go")).unwrap();
        save_item(dir.path(), &task("JOY-0002-BB", "Second to go")).unwrap();
        let mut item = task("JOY-0003-CC", "Refers to both");
        item.parent = Some("JOY-0001-AA".into());
        item.deps = vec!["JOY-0002-BB".into()];
        save_item(dir.path(), &item).unwrap();

        let mut set = load_items(dir.path()).unwrap();
        let (_, reads) = read_count::during(|| {
            delete_item(dir.path(), "JOY-0001-AA").unwrap();
            let updated = remove_references_in(dir.path(), &mut set, "JOY-0001-AA", "m").unwrap();
            assert_eq!(updated, vec!["JOY-0003-CC"]);
            delete_item(dir.path(), "JOY-0002-BB").unwrap();
            let updated = remove_references_in(dir.path(), &mut set, "JOY-0002-BB", "m").unwrap();
            assert_eq!(updated, vec!["JOY-0003-CC"]);
        });
        // delete_item reads the file it deletes; nothing else is read.
        assert_eq!(reads, 2);

        let left = load_items(dir.path()).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, "JOY-0003-CC");
        assert_eq!(left[0].parent, None);
        assert!(left[0].deps.is_empty());
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
