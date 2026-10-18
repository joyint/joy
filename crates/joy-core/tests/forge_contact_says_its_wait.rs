// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! A forge contact says how long joy itself waited before it went out
//! (JP-011F-8E).
//!
//! Every contact to a host stands in joy's line for that host, and the
//! span `forge.contact` covers the wait in that line and the forge's own
//! time together. A week of spans from the platform read "a fetch from
//! codeberg.org takes 2.6 s" while the forge answered in 0.2 s: the rest
//! was the line, and nothing on the span said so. The span now carries
//! `waited_ms`.

joy_test_env::isolate!();

use std::sync::{Arc, Mutex};

/// Keeps what every span says when it is made and when it is recorded to.
#[derive(Clone, Default)]
struct Recorder {
    lines: Arc<Mutex<Vec<String>>>,
}

struct Visitor<'a>(&'a mut String);

impl tracing::field::Visit for Visitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={:?} ", field.name(), value));
    }
}

impl tracing::Subscriber for Recorder {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        let mut text = format!("span {} ", span.metadata().name());
        span.record(&mut Visitor(&mut text));
        self.lines.lock().unwrap().push(text);
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        let mut text = String::from("record ");
        values.record(&mut Visitor(&mut text));
        self.lines.lock().unwrap().push(text);
    }
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, _: &tracing::Event<'_>) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[test]
fn a_contact_says_how_long_it_waited_for_its_turn() {
    // 200 ms per request on this host; a signed-in ls-remote over https
    // is two requests, so the second contact waits about 400 ms
    joy_core::vcs::contact::set_gaps("line.invalid=200");
    let recorder = Recorder::default();
    tracing::subscriber::with_default(recorder.clone(), || {
        for _ in 0..2 {
            joy_core::vcs::contact::run("https://line.invalid/o/r.git", "ls-remote", true, || {
                Ok(())
            })
            .expect("the contact itself does nothing and succeeds");
        }
    });
    joy_core::vcs::contact::set_gaps("");
    let lines = recorder.lines.lock().unwrap().clone();
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with("span forge.contact"))
            .count(),
        2,
        "two contacts: {lines:?}"
    );
    let waits: Vec<u64> = lines
        .iter()
        .filter_map(|l| l.strip_prefix("record waited_ms="))
        .map(|rest| rest.trim().parse().expect("a number of milliseconds"))
        .collect();
    assert_eq!(waits.len(), 2, "each contact says its wait: {lines:?}");
    assert_eq!(waits[0], 0, "the first found the line free");
    assert!(
        (300..=600).contains(&waits[1]),
        "the second waited for the first one's two requests: {} ms",
        waits[1]
    );
}
