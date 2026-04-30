#!/usr/bin/env bats
# JSON output for joy ls (JOY-0123-9A, ADR-036 §1).

load setup

@test "joy ls --json emits valid envelope on empty project" {
    joy init --name "Test" --acronym TST
    run joy ls --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
    echo "$output" | jq -e '.data.items == []' >/dev/null
    echo "$output" | jq -e '.data.total == 0' >/dev/null
}

@test "joy ls --json emits items as structured records" {
    joy init --name "Test" --acronym TST
    joy add task "First"
    joy add bug "Second" --priority high

    run joy ls --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
    echo "$output" | jq -e '.data.total == 2' >/dev/null
    echo "$output" | jq -e '.data.items | length == 2' >/dev/null
    echo "$output" | jq -e '.data.items[0].id | startswith("TST-")' >/dev/null
    echo "$output" | jq -e '[.data.items[].title] | contains(["First","Second"])' >/dev/null
    echo "$output" | jq -e '[.data.items[].type] | contains(["task","bug"])' >/dev/null
}

@test "joy ls --json respects filters" {
    joy init --name "Test" --acronym TST
    joy add task "T1"
    joy add bug "B1"

    run joy ls --type bug --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.total == 1' >/dev/null
    echo "$output" | jq -e '.data.items[0].type == "bug"' >/dev/null
}

@test "joy ls --json hides closed items by default" {
    joy init --name "Test" --acronym TST
    joy auth init --passphrase "$TEST_PASSPHRASE" >/dev/null 2>&1
    joy add task "Active"
    joy add task "Done"
    DONE_ID=$(joy ls 2>/dev/null | grep "Done" | awk '{print $1}')
    joy close "$DONE_ID" >/dev/null 2>&1

    run joy ls --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.total == 1' >/dev/null
    echo "$output" | jq -e '.data.items[0].title == "Active"' >/dev/null
}

@test "joy ls --json --all includes closed items" {
    joy init --name "Test" --acronym TST
    joy auth init --passphrase "$TEST_PASSPHRASE" >/dev/null 2>&1
    joy add task "Active"
    joy add task "Done"
    DONE_ID=$(joy ls 2>/dev/null | grep "Done" | awk '{print $1}')
    joy close "$DONE_ID" >/dev/null 2>&1

    run joy ls --json --all
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.total == 2' >/dev/null
}

@test "joy ls --json position-equivalent: --json before subcommand works" {
    joy init --name "Test" --acronym TST
    joy add task "First"
    run joy --json ls
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
}
