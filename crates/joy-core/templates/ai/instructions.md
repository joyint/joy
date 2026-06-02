# Joy AI Instructions

Run `joy ai tutorial` at the start of every session to load the operational guide. It covers session start (interaction mode, project language, project docs), authentication, the item lifecycle, capabilities and gates, commit messages, and minimum hygiene rules.

If you skipped the tutorial and a write fails with `must authenticate`, you have no session. Run `joy project member`. If you are not listed, ask the operator to run `joy project member add ai:<name>@joy --with-token`. Redeem the token with `joy auth --token <TOKEN> --json`, then pass `--session <session_env>` on every write. See `joy ai tutorial` for the full flow.

Re-read this file and re-run `joy ai tutorial` whenever a `joy` invocation prints `joy X.Y.Z: synced this repo (...)` mentioning either, because operational details may have moved with the version.
