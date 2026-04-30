#!/usr/bin/env bats
# JSON output for joy auth status (JOY-012C-47, ADR-036 §1).

load setup

@test "joy auth status --json before auth init returns auth_initialized=false" {
    joy init --name "Test" --acronym TST
    run joy auth status --json
    [ "$status" -eq 1 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
    echo "$output" | jq -e '.data.authenticated == false' >/dev/null
    echo "$output" | jq -e '.data.auth_initialized == false' >/dev/null
}

@test "joy auth status --json after auth init returns authenticated=true" {
    setup_human_auth
    run joy auth status --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.authenticated == true' >/dev/null
    echo "$output" | jq -e '.data.member == "test@example.com"' >/dev/null
    echo "$output" | jq -e '.data.session_present == true' >/dev/null
    echo "$output" | jq -e '.data.expires_in_seconds | . > 0' >/dev/null
}
