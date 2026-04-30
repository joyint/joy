#!/usr/bin/env bats
# joy auth token add: inline authentication on cold start (JOY-00EF-E5).

load setup

@test "joy auth token add bootstraps session from a single passphrase" {
    setup_human_auth
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE" >/dev/null
    # Drop the session so the next call is a true cold start.
    joy deauth >/dev/null 2>&1 || true
    rm -rf "$XDG_STATE_HOME/joy/sessions"

    run joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"joy_t_"* ]]
    # Session must now exist (created inline by the same command).
    run joy auth status
    [ "$status" -eq 0 ]
}

@test "joy auth token add preserves existing session behaviour" {
    setup_human_auth
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE" >/dev/null
    # Session already exists from setup_human_auth.
    run joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"joy_t_"* ]]
}

@test "joy auth token add fails fast on wrong passphrase even without a session" {
    setup_human_auth
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE" >/dev/null
    rm -rf "$XDG_STATE_HOME/joy/sessions"

    run joy auth token add ai:test@joy --passphrase "wrong-passphrase"
    [ "$status" -ne 0 ]
    [[ "$output" == *"incorrect passphrase"* ]]
    # No session bootstrapped on failure.
    run joy auth status
    [ "$status" -ne 0 ]
}
