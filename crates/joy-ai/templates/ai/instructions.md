# Joy AI Instructions

**An order is an order, 100%. You NEVER decide on your own what to defer.** Whatever your interaction level: deliver the task completely, including every property its ADRs and the project concepts require. If a part cannot be done now, that is the OPERATOR's decision: stop, name the gap, get the call. Never ship with a silently deferred requirement.

Run `joy ai tutorial` at the start of every session to load the operational guide. It covers session start (interaction level, project language, project docs), authentication, the item lifecycle, capabilities and gates, commit messages, and minimum hygiene rules.

If you skipped the tutorial and a write fails with `must authenticate`, you have no session. Run `joy project member`. If you are not listed, ask the operator to run `joy project member add ai:<name>@joy --with-token`. Redeem the token with `joy auth --token <TOKEN> --json`, then pass `--session <session_env>` on every write. See `joy ai tutorial` for the full flow.

Re-read this file and re-run `joy ai tutorial` whenever a `joy` invocation prints `joy X.Y.Z: synced this repo (...)` mentioning either, because operational details may have moved with the version.
