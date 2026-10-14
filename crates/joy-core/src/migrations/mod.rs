// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Schema migrations for joy-managed YAML files.
//!
//! Each migration is a pure transform on `serde_yaml_ng::Value`, isolated
//! in its own module under a date-prefixed filename. Migrations are
//! applied on read; the migrated form persists with the artifact's next
//! regular write — project.yaml at the next `joy auth update` (ADR-035),
//! an item when it is next saved — never as a mass rewrite.
//!
//! Removing a migration after its deprecation window is a one-step
//! operation: delete the module file and the corresponding entry from
//! the area-specific `apply` function. No cross-cutting changes needed.
//!
//! Migrations are grouped by kind so the mechanisms stay separate:
//! - [`project_yaml`]: pure on-read transforms of the parsed YAML value.
//! - [`item_yaml`]: the same discipline for item files.
//! - [`repo`]: filesystem-aware, one-shot reconciles run at sync time.
//! - [`ai_member_name`]: how the name of an AI member is read, in every
//!   file, chat, token and typed word alike.
//! - [`m_2026_10_member_files`]: the members from project.yaml into files
//!   of their own (JI-019D-46). The one migration that needs a person's
//!   key, so it is not applied on read or at sync time: `joy update`
//!   runs it, and joy-cli's own `migrations` module holds the part that
//!   asks for the passphrase. Removing it is deleting the two modules of
//!   that name and their calls in `joy update`.

/// Reading the name of an AI member, whichever form it was written or
/// typed in: THE one way, for every crate and every host.
pub use joy_model::migrations::ai_member_name;

pub mod item_yaml;
pub mod m_2026_10_member_files;
pub mod project_yaml;
pub mod repo;
