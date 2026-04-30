#!/usr/bin/env bats
# JSON output for joy project member (JOY-0139-67, ADR-036 §1).

load setup

@test "joy project member --json emits the members map" {
    setup_human_auth
    run joy project member --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
    echo "$output" | jq -e '.data | has("test@example.com")' >/dev/null
}

@test "joy project member show --json emits id+member" {
    setup_human_auth
    run joy project member show test@example.com --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.id == "test@example.com"' >/dev/null
    echo "$output" | jq -e '.data.member | has("capabilities")' >/dev/null
}

@test "joy project member add --json emits member id + otp" {
    setup_human_auth
    run joy project member add bob@example.com --passphrase "$TEST_PASSPHRASE" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.member == "bob@example.com"' >/dev/null
    echo "$output" | jq -e '.data.otp | length > 0' >/dev/null
}

@test "joy project member rm --json emits removed_member" {
    setup_human_auth
    joy project member add bob@example.com --passphrase "$TEST_PASSPHRASE" >/dev/null
    run joy project member rm bob@example.com --passphrase "$TEST_PASSPHRASE" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.removed_member == "bob@example.com"' >/dev/null
}
