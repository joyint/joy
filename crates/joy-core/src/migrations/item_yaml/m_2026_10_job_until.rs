// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! A job's window ends at `until`.
//!
//! Up to 2026-10 the end of a job's window was written `deadline`; since
//! then it is `until`, the one word on every surface (JOY-02C9-70). A job
//! file from before says `deadline`: here it reads as `until`, and the
//! file says `until` when the job is next saved. That save also lifts the
//! project's format to 3 (JOY-02CA-EE), because a joy from before reads
//! `until` wrong.
//!
//! This is the only place that knows the old word. It can go when no
//! job file says `deadline` any more, which nothing can tell today: a
//! closed job is never saved again and keeps the old key (JOY-02CB-ED).

use serde_yaml_ng::Value;

pub fn migrate(mut value: Value) -> (Value, bool) {
    let moved = (|| {
        let window = value.get_mut("job")?.get_mut("window")?.as_mapping_mut()?;
        let end = window.remove("deadline")?;
        // a file that says both (two branches merged by plain git) keeps
        // what it says under the word of today
        if !window.contains_key("until") {
            window.insert("until".into(), end);
        }
        Some(())
    })();
    (value, moved.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEFORE: &str = "id: T-JOB-0001-AA\ntitle: x\njob:\n  scope: [T-0001-AA]\n  window:\n    not_before: 2026-08-01T00:00:00Z\n    deadline: 2026-09-01T18:00:00Z\n";

    #[test]
    fn a_job_window_from_before_ends_at_until() {
        let (out, changed) = migrate(serde_yaml_ng::from_str(BEFORE).unwrap());
        assert!(changed);
        let window = &out["job"]["window"];
        assert_eq!(window["until"], Value::from("2026-09-01T18:00:00Z"));
        assert!(window.get("deadline").is_none());
        // the start is nobody's business here
        assert_eq!(window["not_before"], Value::from("2026-08-01T00:00:00Z"));

        // a second pass, an item that is no job, a job without a window
        // and a window without an end: nothing to do
        let (again, changed_again) = migrate(out);
        assert!(!changed_again);
        assert_eq!(again["title"], Value::from("x"));
        for text in [
            "id: T-0001-AA\ntitle: x\n",
            "id: T-JOB-0001-AA\ntitle: x\njob:\n  scope: []\n",
            "id: T-JOB-0001-AA\ntitle: x\njob:\n  scope: []\n  window:\n    not_before: 2026-08-01T00:00:00Z\n",
        ] {
            assert!(!migrate(serde_yaml_ng::from_str(text).unwrap()).1, "{text}");
        }
    }

    #[test]
    fn a_file_that_says_both_keeps_until() {
        let text = "id: T-JOB-0001-AA\ntitle: x\njob:\n  scope: []\n  window:\n    deadline: 2026-09-01T18:00:00Z\n    until: 2026-10-01T18:00:00Z\n";
        let (out, changed) = migrate(serde_yaml_ng::from_str(text).unwrap());
        assert!(changed);
        assert_eq!(
            out["job"]["window"]["until"],
            Value::from("2026-10-01T18:00:00Z")
        );
        assert!(out["job"]["window"].get("deadline").is_none());
    }
}
