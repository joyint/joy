// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Schema migrations for joy-managed YAML files.
//!
//! Each migration is a pure transform on `serde_yaml_ng::Value`, isolated
//! in its own module under a date-prefixed filename. Migrations are
//! applied on read; persistence happens at the next `joy auth update`,
//! never as a silent on-save rewrite (per ADR-035).
//!
//! Removing a migration after its deprecation window is a one-step
//! operation: delete the module file and the corresponding entry from
//! the area-specific `apply` function. No cross-cutting changes needed.
//!
//! Migrations are grouped by kind so the mechanisms stay separate:
//! - [`project_yaml`]: pure on-read transforms of the parsed YAML value.
//! - [`item_yaml`]: the same discipline for item files.
//! - [`repo`]: filesystem-aware, one-shot reconciles run at sync time.

pub mod item_yaml;
pub mod project_yaml;
pub mod repo;
