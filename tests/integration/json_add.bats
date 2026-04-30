#!/usr/bin/env bats
# JSON output for joy add (JOY-012F-5D, ADR-036 §1).

load setup

@test "joy add --json emits the created item" {
    joy init --name "Test" --acronym TST
    run joy add task "Bug fix" --priority high --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
    echo "$output" | jq -e '.data.id | startswith("TST-")' >/dev/null
    echo "$output" | jq -e '.data.title == "Bug fix"' >/dev/null
    echo "$output" | jq -e '.data.type == "task"' >/dev/null
    echo "$output" | jq -e '.data.priority == "high"' >/dev/null
    echo "$output" | jq -e '.data.status == "new"' >/dev/null
}
