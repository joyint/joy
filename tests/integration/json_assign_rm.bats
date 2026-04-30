#!/usr/bin/env bats
# JSON output for joy assign + joy rm (JOY-0132-84, JOY-0133-1A, ADR-036 §1).

load setup

@test "joy assign --json emits the item with new assignee" {
    setup_human_auth
    joy add task "Work"
    ID=$(joy ls 2>/dev/null | grep Work | awk '{print $1}')
    run joy assign "$ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.assignees | length == 1' >/dev/null
    echo "$output" | jq -e '.data.assignees[0].member == "test@example.com"' >/dev/null
}

@test "joy rm --json emits a list of deleted items" {
    setup_human_auth
    joy add task "Doomed"
    ID=$(joy ls 2>/dev/null | grep Doomed | awk '{print $1}')
    run joy rm "$ID" --force --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
    echo "$output" | jq -e '.data.deleted | length == 1' >/dev/null
    echo "$output" | jq -e ".data.deleted[0].id == \"$ID\"" >/dev/null
    echo "$output" | jq -e '.data.deleted[0].title == "Doomed"' >/dev/null
}
