#!/usr/bin/env bats
# JSON output for joy project (read paths) (JOY-012A-4D, ADR-036 §1).

load setup

@test "joy project --json emits the full project object" {
    joy init --name "Demo" --acronym DM
    run joy project --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
    echo "$output" | jq -e '.data.name == "Demo"' >/dev/null
    echo "$output" | jq -e '.data.acronym == "DM"' >/dev/null
    echo "$output" | jq -e '.data | has("members")' >/dev/null
    echo "$output" | jq -e '.data | has("language")' >/dev/null
}

@test "joy project get --json emits {key, value}" {
    joy init --name "Demo" --acronym DM
    run joy project get name --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.key == "name"' >/dev/null
    echo "$output" | jq -e '.data.value == "Demo"' >/dev/null
}

@test "joy project get --json on unset optional emits null value, not exit 1" {
    joy init --name "Demo" --acronym DM
    run joy project get description --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.key == "description"' >/dev/null
    echo "$output" | jq -e '.data.value == null' >/dev/null
}
