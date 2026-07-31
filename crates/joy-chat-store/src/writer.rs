// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Whose identity this process writes chats as.
//!
//! A sealed chat can only be read and written by someone who holds a key
//! slot in it, so every read and every write needs an identity seed. This
//! module carries that seed for the current process (CLI, desktop) or,
//! per request, for the current thread (a server handling many callers).
//!
//! It replaces the former "custodian" seed, which carried the same value
//! under a name that suggested a party standing above the chat. There is
//! no such party any more: a seed here belongs to a member, and it opens
//! exactly the chats that member is in (JI-0174 family).
//!
//! This is a way station. With the split of joy-chat into the chat itself
//! and its git storage (JAPP-0135-FD), the seed becomes an argument of
//! the few functions that need it and this module goes away. It is kept
//! for now so the change lands in one piece instead of rippling through
//! every call site twice.

/// The process-wide seed: whoever holds it reads and writes their chats.
/// None (the default) means nobody is authenticated here, so sealed chats
/// stay sealed and cannot be written.
static PROCESS_SEED: std::sync::RwLock<Option<[u8; 32]>> = std::sync::RwLock::new(None);

thread_local! {
    /// A per-thread override. When set (outer `Some`) it WINS over the
    /// process seed for this thread, so a server can decide per request
    /// exactly as it does for zone keys: a locked session installs
    /// `Some(None)` and fails closed. CLI and desktop never set it.
    static THREAD_SEED: std::cell::Cell<Option<Option<[u8; 32]>>> =
        const { std::cell::Cell::new(None) };
}

/// Set (or clear) the process-wide seed.
pub fn set_seed(seed: Option<[u8; 32]>) {
    *PROCESS_SEED.write().unwrap_or_else(|e| e.into_inner()) = seed;
}

/// Install (outer `Some`) or drop (`None`) the per-thread override. A
/// server sets it around each request and clears it afterwards, because
/// blocking threads are reused.
pub fn set_thread_seed(value: Option<Option<[u8; 32]>>) {
    THREAD_SEED.with(|c| c.set(value));
}

/// The seed in force here: the thread override if one is installed,
/// otherwise the process-wide seed.
pub fn seed() -> Option<[u8; 32]> {
    if let Some(thread) = THREAD_SEED.with(std::cell::Cell::get) {
        return thread;
    }
    *PROCESS_SEED.read().unwrap_or_else(|e| e.into_inner())
}
