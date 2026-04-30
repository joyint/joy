#!/usr/bin/env bats
# joy auth status surfaces delegated AI sessions (JOY-00F2-A6).

load setup

@test "joy auth status lists delegated AI sessions in display mode" {
    setup_human_auth
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE" >/dev/null
    joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE" >/dev/null

    run joy auth status
    [ "$status" -eq 0 ]
    [[ "$output" == *"Auth Status"* ]]
    [[ "$output" == *"Your session"* ]]
    [[ "$output" == *"Delegated AI sessions"* ]]
    [[ "$output" == *"ai:test@joy"* ]]
}

@test "joy auth status --json includes delegated_sessions array" {
    setup_human_auth
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE" >/dev/null
    joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE" >/dev/null

    run joy auth status --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data | has("delegated_sessions")' >/dev/null
    echo "$output" | jq -e '.data.delegated_sessions | length == 1' >/dev/null
    echo "$output" | jq -e '.data.delegated_sessions[0].member == "ai:test@joy"' >/dev/null
}

@test "joy auth status omits the delegated section when nothing was delegated" {
    setup_human_auth
    run joy auth status
    [ "$status" -eq 0 ]
    [[ "$output" != *"Delegated AI sessions"* ]]
}
