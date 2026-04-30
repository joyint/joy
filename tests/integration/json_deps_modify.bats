#!/usr/bin/env bats
# JSON output for joy deps --add/--rm (JOY-0135-8A, ADR-036 §1).

load setup

@test "joy deps --add --json emits the updated item with deps" {
    setup_human_auth
    joy add task "Dep"
    DEP=$(joy ls 2>/dev/null | grep "^TP" | head -1 | awk '{print $1}')
    joy add task "Consumer"
    CONS=$(joy ls 2>/dev/null | grep Consumer | awk '{print $1}')
    run joy deps "$CONS" --add "$DEP" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e ".data.id == \"$CONS\"" >/dev/null
    echo "$output" | jq -e ".data.deps == [\"$DEP\"]" >/dev/null
}

@test "joy deps --rm --json emits the updated item without the dep" {
    setup_human_auth
    joy add task "Dep"
    DEP=$(joy ls 2>/dev/null | grep "^TP" | head -1 | awk '{print $1}')
    joy add task "Consumer"
    CONS=$(joy ls 2>/dev/null | grep Consumer | awk '{print $1}')
    joy deps "$CONS" --add "$DEP" >/dev/null
    run joy deps "$CONS" --rm "$DEP" --json
    [ "$status" -eq 0 ]
    # deps elided by serde when empty; absence is equivalent to []
    echo "$output" | jq -e '(.data.deps // []) == []' >/dev/null
}
