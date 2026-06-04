#!/usr/bin/env bats
# The -D / --decisions view: filter to decisions, show the val column,
# and a board grouped by validity instead of status (JOY-01B5-21).

load setup

@test "joy ls -D filters to decisions and shows the VAL column" {
    setup_human_auth
    joy add decision "A weighty decision"
    joy add task "An unrelated task"
    run joy ls -D
    [ "$status" -eq 0 ]
    [[ "$output" == *"VAL"* ]]
    [[ "$output" == *"A weighty decision"* ]]
    [[ "$output" != *"An unrelated task"* ]]
}

@test "joy ls -D is the long form --decisions too" {
    setup_human_auth
    joy add decision "Long form decision"
    run joy ls --decisions
    [ "$status" -eq 0 ]
    [[ "$output" == *"Long form decision"* ]]
}

@test "joy board -D groups decisions by validity" {
    setup_human_auth
    joy add decision "Accepted rule"
    ID=$(joy ls -D 2>/dev/null | grep "Accepted rule" | awk '{print $1}')
    joy close "$ID"
    run joy board -D --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.columns[] | select(.status == "ACCEPTED") | .count == 1' >/dev/null
}

@test "joy ls -c val shows the validity column without -D" {
    setup_human_auth
    joy add decision "Column via key"
    run joy ls -c val
    [ "$status" -eq 0 ]
    [[ "$output" == *"VAL"* ]]
}
