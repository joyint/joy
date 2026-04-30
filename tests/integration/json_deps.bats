#!/usr/bin/env bats
# JSON output for joy deps (read path) (JOY-0126-7C, ADR-036 §1).

load setup

@test "joy deps --json emits resolved deps" {
    joy init --name "Test" --acronym TST
    joy add task "Dep"
    DEP_ID=$(joy ls 2>/dev/null | grep "^TST" | head -1 | awk '{print $1}')
    joy add task "Consumer"
    CONS_ID=$(joy ls 2>/dev/null | grep Consumer | awk '{print $1}')
    joy deps "$CONS_ID" --add "$DEP_ID" >/dev/null

    run joy deps "$CONS_ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
    echo "$output" | jq -e ".data.id == \"$CONS_ID\"" >/dev/null
    echo "$output" | jq -e '.data.deps | length == 1' >/dev/null
    echo "$output" | jq -e '.data.deps[0].found == true' >/dev/null
    echo "$output" | jq -e '.data.deps[0].title == "Dep"' >/dev/null
}

@test "joy deps --json on no-deps item emits empty array" {
    joy init --name "Test" --acronym TST
    joy add task "Lonely"
    ID=$(joy ls 2>/dev/null | grep Lonely | awk '{print $1}')
    run joy deps "$ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.deps == []' >/dev/null
}
