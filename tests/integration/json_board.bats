#!/usr/bin/env bats
# JSON output for joy board (JOY-0125-80, ADR-036 §1).

load setup

@test "joy board --json emits all status columns" {
    joy init --name "Test" --acronym TST
    joy add task "T1"

    run joy board --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
    echo "$output" | jq -e '.data.columns | length == 6' >/dev/null
    echo "$output" | jq -e '[.data.columns[].status] == ["NEW","OPEN","IN-PROGRESS","REVIEW","CLOSED","DEFERRED"]' >/dev/null
}

@test "joy board --json puts items in their status column" {
    joy init --name "Test" --acronym TST
    joy auth init --passphrase "$TEST_PASSPHRASE" >/dev/null 2>&1
    joy add task "Active"
    joy add task "Done"
    DONE_ID=$(joy ls 2>/dev/null | grep "Done" | awk '{print $1}')
    joy close "$DONE_ID" >/dev/null 2>&1

    run joy board --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.columns[] | select(.status == "NEW") | .count == 1' >/dev/null
    echo "$output" | jq -e '.data.columns[] | select(.status == "CLOSED") | .count == 1' >/dev/null
    echo "$output" | jq -e '.data.columns[] | select(.status == "NEW") | .items[0].title == "Active"' >/dev/null
}

@test "joy board --json on empty project is a valid envelope" {
    joy init --name "Test" --acronym TST
    run joy board --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
    echo "$output" | jq -e '.data.columns == []' >/dev/null
}
