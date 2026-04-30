#!/usr/bin/env bats
# JSON output for joy milestone add/edit/rm/link/unlink (JOY-0136-D2, ADR-036 §1).

load setup

@test "joy milestone add --json emits the new milestone" {
    setup_human_auth
    run joy milestone add "Sprint" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data | has("id")' >/dev/null
    echo "$output" | jq -e '.data.title == "Sprint"' >/dev/null
}

@test "joy milestone link --json emits the updated item" {
    setup_human_auth
    MS=$(joy milestone add "S1" 2>&1 | grep -oE 'TP-MS-[0-9A-F]+(-[0-9A-F]+)?' | head -1)
    joy add task "Linked"
    ID=$(joy ls 2>/dev/null | grep Linked | awk '{print $1}')

    run joy milestone link "$ID" "$MS" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e ".data.id == \"$ID\"" >/dev/null
    echo "$output" | jq -e ".data.milestone == \"$MS\"" >/dev/null
}

@test "joy milestone unlink --json emits the updated item" {
    setup_human_auth
    MS=$(joy milestone add "S1" 2>&1 | grep -oE 'TP-MS-[0-9A-F]+(-[0-9A-F]+)?' | head -1)
    joy add task "Linked"
    ID=$(joy ls 2>/dev/null | grep Linked | awk '{print $1}')
    joy milestone link "$ID" "$MS" >/dev/null

    run joy milestone unlink "$ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '(.data.milestone // null) == null' >/dev/null
}
