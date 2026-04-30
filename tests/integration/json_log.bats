#!/usr/bin/env bats
# JSON output for joy log (JOY-0128-6A, ADR-036 §1).

load setup

@test "joy log --json emits envelope of events" {
    setup_human_auth
    joy add task "First"
    ID=$(joy ls 2>/dev/null | grep First | awk '{print $1}')
    joy comment "$ID" "hello"

    run joy log --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
    echo "$output" | jq -e '.data.total | . > 0' >/dev/null
    echo "$output" | jq -e '.data.events[0] | has("timestamp")' >/dev/null
    echo "$output" | jq -e '.data.events[0] | has("event_type")' >/dev/null
    echo "$output" | jq -e '.data.events[0] | has("user")' >/dev/null
}

@test "joy log --json on empty project returns empty events" {
    joy init --name "Test" --acronym TST
    run joy log --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.events == []' >/dev/null
    echo "$output" | jq -e '.data.has_more == false' >/dev/null
}
