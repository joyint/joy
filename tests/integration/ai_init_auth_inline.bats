#!/usr/bin/env bats
# joy ai init: inline auth bootstrap when caller has no registered key.

load setup

@test "joy ai init bootstraps auth inline on cold start" {
    joy init --name "Test Project" 2>/dev/null
    # No `joy auth init` here; ai init must do it.
    # Suppress tool detection so the test does not depend on which AI
    # binaries happen to be installed on the host; we only care that
    # the auth bootstrap step ran before any other AI Init phase.
    # `</dev/null` keeps the per-doc-template and per-tool
    # `[Y/n]` prompts from blocking when the caller's stdin is a TTY
    # (e.g. interactive `just check`). With empty stdin they fall
    # through to their Default-Y branch.
    PATH_OVERRIDE="$(dirname "$JOY_BIN"):/usr/bin:/bin"
    run env PATH="$PATH_OVERRIDE" joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null
    [ "$status" -eq 0 ]
    [[ "$output" == *"Authentication"* ]]
    [[ "$output" == *"Authentication initialized for"* ]]
    [[ "$output" == *"Public key registered"* ]]
    # Persistent indicator: the caller now has a verify_key in
    # project.yaml. We deliberately do NOT run `joy auth status`
    # here: that checks the *session*, which is TTY-bound by design
    # (identity::check_session) and would not match the non-TTY
    # context we just used to drive the init through its [Y/n]
    # prompts. The auth-bootstrap claim is fully tied to verify_key.
    grep -q "^    verify_key:" .joy/project.yaml
}

@test "joy ai init skips auth section when caller is already authenticated" {
    setup_human_auth
    run joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null
    [ "$status" -eq 0 ]
    # The auth section header must NOT appear when already initialised.
    [[ "$output" != *"Setting up authentication"* ]]
    [[ "$output" != *"RECOVERY KEY"* ]]
}

@test "joy ai init succeeds in an anonymous project (founder keyed by opaque id)" {
    # Regression: in anonymous mode (ADR-042) the member map is keyed by an
    # opaque id, not the cleartext e-mail. A direct `members.get(&email)`
    # lookup wrongly reported the founder as "not a registered project member"
    # right after `joy init --anonymous`. ai init must resolve via the
    # privacy-aware member_key_for_email instead.
    joy init --name "Test Project" --anonymous --passphrase "$TEST_PASSPHRASE" 2>/dev/null
    # The founder is already authenticated by the anonymous init, so ai init
    # must NOT re-prompt for auth and must NOT fail on member resolution.
    PATH_OVERRIDE="$(dirname "$JOY_BIN"):/usr/bin:/bin"
    run env PATH="$PATH_OVERRIDE" joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null
    [ "$status" -eq 0 ]
    [[ "$output" != *"not a registered project member"* ]]
    # Auth was already initialised by `init --anonymous`, so the auth-bootstrap
    # section must be skipped.
    [[ "$output" != *"Setting up authentication"* ]]
    # The cleartext e-mail must never appear in the committed project.yaml.
    ! grep -q "test@example.com" .joy/project.yaml
}

@test "joy ai init fails clearly when caller is not a project member" {
    joy init --name "Test Project" 2>/dev/null
    # Switch git identity to someone who has never been added to the project.
    git config user.email "stranger@example.com"
    PATH_OVERRIDE="$(dirname "$JOY_BIN"):/usr/bin:/bin"
    run env PATH="$PATH_OVERRIDE" joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null
    [ "$status" -ne 0 ]
    [[ "$output" == *"stranger@example.com"* ]]
    [[ "$output" == *"not a registered project member"* ]]
}
