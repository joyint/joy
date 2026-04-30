#!/usr/bin/env bats
# JSON output for joy edit (JOY-0130-B8, ADR-036 §1).

load setup

@test "joy edit --json emits the updated item" {
    setup_human_auth
    joy add task "Original"
    ID=$(joy ls 2>/dev/null | grep Original | awk '{print $1}')
    run joy edit "$ID" --priority high --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.priority == "high"' >/dev/null
    echo "$output" | jq -e ".data.id == \"$ID\"" >/dev/null
}

@test "joy edit --json with no changes still emits the current item" {
    setup_human_auth
    joy add task "Stable"
    ID=$(joy ls 2>/dev/null | grep Stable | awk '{print $1}')
    run joy edit "$ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e ".data.id == \"$ID\"" >/dev/null
}
