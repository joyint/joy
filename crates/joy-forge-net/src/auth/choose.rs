// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Which login answers for a remote (D4.1c).
//!
//! This is not academic. gh keeps several accounts per host and
//! documents the trap itself: "Without the --user flag, the active
//! account for the host is chosen." A connector that calls
//! `gh auth token --hostname H` and nothing else hands back whichever
//! account the person last switched to, which is how a private
//! repository gets pushed under a work login. tea is multi login per
//! host too; glab holds one token per host block, so there the rule
//! collapses to step 3.
//!
//! The order, and every answer says which step decided:
//!
//! 1. the device local pin for that host on that project;
//! 2. the login the memory recorded for this remote;
//! 3. the only login the host holds, with no probe at all;
//! 4. a probe: one REST call for `owner/repo` per candidate, in the
//!    order plugin keychain login, then the forge CLI's active login,
//!    then the rest in a stable order; the first that answers, and for
//!    a push direction reports write, wins;
//! 5. otherwise `no-login-for-repo`, with the sentence that names the
//!    logins that were tried.

use super::ChoseBy;

/// Steps 1 to 3: the ones that spend no request at all. `cli` is the
/// `--login` of the call, which is a pin the caller states directly.
pub fn without_probe(
    cli: Option<&str>,
    pin: Option<&str>,
    memory: Option<&str>,
    known: &[String],
) -> Option<(String, ChoseBy)> {
    if let Some(login) = non_empty(cli).or_else(|| non_empty(pin)) {
        return Some((login, ChoseBy::Pin));
    }
    // A remembered login that the host no longer holds is not an
    // answer: the entry was removed, and the memory is stale.
    if let Some(login) = non_empty(memory).filter(|login| known.iter().any(|k| k == login)) {
        return Some((login, ChoseBy::Memory));
    }
    match known {
        [only] => Some((only.clone(), ChoseBy::Only)),
        _ => None,
    }
}

/// Step 4's candidate order: the connector's own logins first, then the
/// forge CLI's, then the rest, each named once.
pub fn probe_order(own: &[String], foreign: &[String]) -> Vec<String> {
    let mut order: Vec<String> = Vec::new();
    for login in own.iter().chain(foreign.iter()) {
        let login = login.trim();
        if !login.is_empty() && !order.iter().any(|known| known == login) {
            order.push(login.to_string());
        }
    }
    order
}

/// The answer of step 5, with the sentence D4.1c writes: "None of your
/// GitHub logins (work, scotty) can reach acme/widgets. Sign in with
/// the login that can."
///
/// `push` is the direction the call asked for. A login that reads a
/// repository it may not push to is not an answer for a push, and
/// saying it "cannot reach" the repository would send the person
/// looking for the wrong thing.
pub fn no_login_for_repo(
    display: &str,
    logins: &[String],
    repo_path: &str,
    push: bool,
) -> serde_json::Value {
    let mut answer = serde_json::json!({
        "known": false,
        "reason": "no-login-for-repo",
    });
    if !logins.is_empty() {
        let what = if push { "can push to" } else { "can reach" };
        answer["message"] = serde_json::Value::String(format!(
            "None of your {display} logins ({}) {what} {repo_path}. \
             Sign in with the login that can.",
            logins.join(", ")
        ));
    }
    answer
}

/// The answer of D2.7c when no login can reach the repository because
/// the OWNER organisation has not approved Joy.
///
/// This is not step 5 and must never be dressed as one: "None of your
/// logins can reach acme/widgets. Sign in with the login that can."
/// sends a person to sign in again with an account that will be refused
/// for exactly the same reason, while the one action that helps belongs
/// to somebody else, the organisation's owner, on one page
/// (JOY-02A9-48).
pub fn needs_org_approval(owner: &str, repo_path: &str, url: Option<&str>) -> serde_json::Value {
    let mut answer = serde_json::json!({
        "known": false,
        "reason": "needs_org_approval",
        "message": format!(
            "Your organisation must approve Joy for this repository. \
             {owner} has OAuth app access restrictions switched on, so no login of yours \
             reaches {repo_path} until an owner of {owner} approves Joy."
        ),
    });
    if let Some(url) = url.map(str::trim).filter(|url| !url.is_empty()) {
        answer["action"] = serde_json::Value::String(url.to_string());
    }
    answer
}

/// The `owner` of an `owner/repo` path, which is the organisation an
/// approval is asked of.
pub fn owner_of(repo_path: &str) -> &str {
    repo_path
        .trim_matches('/')
        .split('/')
        .next()
        .unwrap_or_default()
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logins(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    /// The order is the order, and each step names itself.
    #[test]
    fn the_pin_wins_then_the_memory_then_the_only_login() {
        let known = logins(&["work", "scotty"]);
        assert_eq!(
            without_probe(Some("work"), Some("scotty"), Some("scotty"), &known),
            Some(("work".to_string(), ChoseBy::Pin))
        );
        assert_eq!(
            without_probe(None, Some("scotty"), Some("work"), &known),
            Some(("scotty".to_string(), ChoseBy::Pin))
        );
        assert_eq!(
            without_probe(None, None, Some("work"), &known),
            Some(("work".to_string(), ChoseBy::Memory))
        );
        // two logins, no pin, no memory: this is what the probe is for
        assert_eq!(without_probe(None, None, None, &known), None);
        assert_eq!(
            without_probe(None, None, None, &logins(&["scotty"])),
            Some(("scotty".to_string(), ChoseBy::Only))
        );
        assert_eq!(without_probe(None, None, None, &[]), None);
    }

    /// A memory of a login the host no longer holds must not outrank
    /// the only login that is left.
    #[test]
    fn a_memory_of_a_removed_login_is_not_an_answer() {
        let known = logins(&["scotty"]);
        assert_eq!(
            without_probe(None, None, Some("work"), &known),
            Some(("scotty".to_string(), ChoseBy::Only))
        );
        assert_eq!(without_probe(None, None, Some("work"), &[]), None);
    }

    #[test]
    fn the_probe_asks_the_connectors_own_logins_first_and_each_login_once() {
        let order = probe_order(&logins(&["scotty", "work"]), &logins(&["work", "ci"]));
        assert_eq!(order, logins(&["scotty", "work", "ci"]));
        assert!(probe_order(&[], &[]).is_empty());
    }

    #[test]
    fn the_last_step_names_the_logins_that_could_not_reach_the_repository() {
        let answer = no_login_for_repo(
            "GitHub",
            &logins(&["work", "scotty"]),
            "acme/widgets",
            false,
        );
        assert_eq!(answer["known"], false);
        assert_eq!(answer["reason"], "no-login-for-repo");
        let message = answer["message"].as_str().unwrap();
        assert!(message.contains("work, scotty"), "{message}");
        assert!(message.contains("acme/widgets"), "{message}");
        assert!(message.contains("can reach"), "{message}");
        // a push asked for more than reading, and says so
        let pushing = no_login_for_repo("GitHub", &logins(&["work"]), "acme/widgets", true);
        let message = pushing["message"].as_str().unwrap();
        assert!(message.contains("can push to acme/widgets"), "{message}");
        // a host with no logins at all has nothing to name
        assert!(no_login_for_repo("GitHub", &[], "acme/widgets", false)
            .get("message")
            .is_none());
    }
}
