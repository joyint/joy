#!/usr/bin/env bats
# joy config get: wildcard listings and --describe semantic context
# (JOY-0187-D0).

load setup

@test "joy config get <prefix>.* lists every leaf under the prefix" {
    joy init --name "T" >/dev/null
    run joy config get output.*
    [ "$status" -eq 0 ]
    [[ "$output" == *"output.color"* ]]
    [[ "$output" == *"output.emoji"* ]]
    [[ "$output" == *"output.fortune"* ]]
    [[ "$output" == *"output.short"* ]]
}

@test "joy config get --describe adds a one-line description to scalar values" {
    joy init --name "T" >/dev/null
    run joy config get interaction.default --describe
    [ "$status" -eq 0 ]
    [[ "$output" == *"collaborative"* ]]
    [[ "$output" == *"propose approach"* ]]
}

@test "joy config get <prefix>.* --describe annotates each leaf" {
    joy init --name "T" >/dev/null
    run joy config get interaction.* --describe
    [ "$status" -eq 0 ]
    [[ "$output" == *"interaction.default"* ]]
    [[ "$output" == *"collaborative"* ]]
    [[ "$output" == *"propose approach"* ]]
}

@test "joy config get <prefix>.* --describe --json returns entries array" {
    joy init --name "T" >/dev/null
    run joy config get output.* --describe --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.key == "output.*"' >/dev/null
    echo "$output" | jq -e '.data.entries | type == "array"' >/dev/null
    echo "$output" | jq -e '.data.entries | map(.key) | contains(["output.color"])' >/dev/null
    # output.color value `auto` has a known description.
    echo "$output" | jq -e '.data.entries[] | select(.key == "output.color") | .description != null' >/dev/null
}

@test "joy config get key --describe --json adds description on scalar" {
    joy init --name "T" >/dev/null
    run joy config get workflow.auto-git --describe --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.value == "add"' >/dev/null
    echo "$output" | jq -e '.data.description | type == "string"' >/dev/null
}

@test "joy config get <prefix> (no wildcard) keeps the legacy object payload" {
    joy init --name "T" >/dev/null
    # The bare prefix returns the whole section as before -- no API break.
    run joy config get output --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.key == "output"' >/dev/null
    echo "$output" | jq -e '.data.value | type == "object"' >/dev/null
    # Wildcard form is opt-in.
    [ "$(echo "$output" | jq 'has("entries")')" = "false" ]
}

@test "joy config get <unknown>.* exits non-zero on empty match" {
    joy init --name "T" >/dev/null
    run joy config get nonexistent.*
    [ "$status" -ne 0 ]
}
