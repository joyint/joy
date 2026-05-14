#!/usr/bin/env bats
# joy ai init: inline auth bootstrap when caller has no registered key.

load setup

@test "joy ai init bootstraps auth inline on cold start" {
    joy init --name "Test Project" 2>/dev/null
    # No `joy auth init` here; ai init must do it.
    # Suppress tool detection so the test does not depend on which AI
    # binaries happen to be installed on the host; we only care that
    # the auth bootstrap step ran before any other AI Init phase.
    run env PATH="$(dirname "$JOY_BIN"):/usr/bin:/bin" \
        joy ai init --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Authentication"* ]]
    [[ "$output" == *"Authentication initialized for"* ]]
    [[ "$output" == *"Public key registered"* ]]
    # Caller now has a verify_key in project.yaml.
    run joy auth status
    [ "$status" -eq 0 ]
}

@test "joy ai init skips auth section when caller is already authenticated" {
    setup_human_auth
    run joy ai init --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    # The auth section header must NOT appear when already initialised.
    [[ "$output" != *"Setting up authentication"* ]]
    [[ "$output" != *"RECOVERY KEY"* ]]
}

@test "joy ai init fails clearly when caller is not a project member" {
    joy init --name "Test Project" 2>/dev/null
    # Switch git identity to someone who has never been added to the project.
    git config user.email "stranger@example.com"
    run env PATH="$(dirname "$JOY_BIN"):/usr/bin:/bin" \
        joy ai init --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"stranger@example.com"* ]]
    [[ "$output" == *"not a registered project member"* ]]
}
