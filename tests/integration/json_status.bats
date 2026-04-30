#!/usr/bin/env bats
# JSON output for joy status (start/submit/close/reopen) (JOY-0134-67, ADR-036 §1).

load setup

@test "joy start --json emits a transition payload" {
    setup_human_auth
    joy add task "Work"
    ID=$(joy ls 2>/dev/null | grep Work | awk '{print $1}')
    run joy start "$ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e ".data.id == \"$ID\"" >/dev/null
    echo "$output" | jq -e '.data.from == "new"' >/dev/null
    echo "$output" | jq -e '.data.to == "in-progress"' >/dev/null
    echo "$output" | jq -e '.data.auto_closed == []' >/dev/null
}

@test "joy close --json includes auto-closed parent in payload" {
    setup_human_auth
    joy add task "Parent"
    PARENT=$(joy ls 2>/dev/null | grep Parent | awk '{print $1}')
    joy add task "Child" --parent "$PARENT"
    CHILD=$(joy ls 2>/dev/null | grep Child | awk '{print $1}')

    run joy close "$CHILD" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.to == "closed"' >/dev/null
    echo "$output" | jq -e '.data.auto_closed | length == 1' >/dev/null
    echo "$output" | jq -e ".data.auto_closed[0].id == \"$PARENT\"" >/dev/null
    echo "$output" | jq -e '.data.auto_closed[0].to == "closed"' >/dev/null
}
