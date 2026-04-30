#!/usr/bin/env bats
# JSON output for joy comment (JOY-0131-D9, ADR-036 §1).

load setup

@test "joy comment --json emits the item with the new comment" {
    setup_human_auth
    joy add task "Talk"
    ID=$(joy ls 2>/dev/null | grep Talk | awk '{print $1}')
    run joy comment "$ID" "First line" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.comments | length == 1' >/dev/null
    echo "$output" | jq -e '.data.comments[0].text == "First line"' >/dev/null
}
