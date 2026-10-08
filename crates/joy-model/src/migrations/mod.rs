// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Migrations applied on read that every crate shares, the browser
//! included. The others live in `joy_core::migrations`, which hands
//! these on.

pub mod ai_member_name;
