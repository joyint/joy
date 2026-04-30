#!/usr/bin/env bats
# JSON output for joy show (JOY-0124-97, ADR-036 §1).

load setup

@test "joy show --json emits the item as the data payload" {
    joy init --name "Test" --acronym TST
    joy add task "First" --priority high
    ID=$(joy ls 2>/dev/null | grep First | awk '{print $1}')

    run joy show "$ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
    echo "$output" | jq -e ".data.id == \"$ID\"" >/dev/null
    echo "$output" | jq -e '.data.title == "First"' >/dev/null
    echo "$output" | jq -e '.data.type == "task"' >/dev/null
    echo "$output" | jq -e '.data.priority == "high"' >/dev/null
    echo "$output" | jq -e '.data.status == "new"' >/dev/null
}

@test "joy show --json includes comments" {
    joy init --name "Test" --acronym TST
    joy add task "Commented"
    ID=$(joy ls 2>/dev/null | grep Commented | awk '{print $1}')
    joy comment "$ID" "First comment"
    joy comment "$ID" "Second comment"

    run joy show "$ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.comments | length == 2' >/dev/null
    echo "$output" | jq -e '.data.comments[0].text == "First comment"' >/dev/null
}

@test "joy show --json fails cleanly on unknown item" {
    joy init --name "Test" --acronym TST
    run joy show TST-9999 --json
    [ "$status" -ne 0 ]
}
