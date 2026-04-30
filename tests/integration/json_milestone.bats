#!/usr/bin/env bats
# JSON output for joy milestone (read paths) (JOY-0129-D6, ADR-036 §1).

load setup

@test "joy milestone ls --json emits envelope" {
    setup_human_auth
    joy milestone add "First Sprint"
    joy milestone add "Second Sprint"

    run joy milestone ls --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
    echo "$output" | jq -e '.data.total == 2' >/dev/null
    echo "$output" | jq -e '.data.milestones | length == 2' >/dev/null
    echo "$output" | jq -e '[.data.milestones[].title] | contains(["First Sprint","Second Sprint"])' >/dev/null
}

@test "joy milestone show --json emits milestone with linked items" {
    setup_human_auth
    MS=$(joy milestone add "First" 2>&1 | grep -oE 'TP-MS-[0-9A-F]+(-[0-9A-F]+)?' | head -1)
    joy add task "Linked"
    ID=$(joy ls 2>/dev/null | grep Linked | awk '{print $1}')
    joy milestone link "$ID" "$MS"

    run joy milestone show "$MS" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
    echo "$output" | jq -e ".data.id == \"$MS\"" >/dev/null
    echo "$output" | jq -e '.data.title == "First"' >/dev/null
    echo "$output" | jq -e '.data.total == 1' >/dev/null
    echo "$output" | jq -e '.data.items[0].title == "Linked"' >/dev/null
}

@test "joy milestone ls --json on empty project emits empty array" {
    joy init --name "Test" --acronym TST
    run joy milestone ls --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.milestones == []' >/dev/null
}
