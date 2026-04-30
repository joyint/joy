#!/usr/bin/env bats
# JSON output for joy find (JOY-0127-F0, ADR-036 §1).

load setup

@test "joy find --json emits matching items" {
    joy init --name "Test" --acronym TST
    joy add task "Login flow"
    joy add task "Logout flow"
    joy add bug "Unrelated"

    run joy find login --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
    echo "$output" | jq -e '.data.query == "login"' >/dev/null
    echo "$output" | jq -e '.data.total == 1' >/dev/null
    echo "$output" | jq -e '.data.items[0].title == "Login flow"' >/dev/null
}

@test "joy find --json on no matches returns empty" {
    joy init --name "Test" --acronym TST
    joy add task "First"
    run joy find xyzabc --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.total == 0' >/dev/null
    echo "$output" | jq -e '.data.items == []' >/dev/null
}
