#!/usr/bin/env bats
# JSON output for joy config (read paths) (JOY-012B-B5, ADR-036 §1).

load setup

@test "joy config --json emits effective + personal" {
    joy init --name "Test" --acronym TST
    run joy config --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
    echo "$output" | jq -e '.data | has("effective")' >/dev/null
    echo "$output" | jq -e '.data | has("personal")' >/dev/null
}

@test "joy config get --json returns key/value" {
    joy init --name "Test" --acronym TST
    run joy config get output --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.key == "output"' >/dev/null
    echo "$output" | jq -e '.data.value | type == "object"' >/dev/null
}

@test "joy config get --json on unknown key returns null instead of exit 1" {
    joy init --name "Test" --acronym TST
    run joy config get nonexistent.key --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.value == null' >/dev/null
}
