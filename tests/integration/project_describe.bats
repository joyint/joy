#!/usr/bin/env bats
# joy project get: wildcard listings + --describe semantic context.

load setup

@test "joy project get <prefix>.* lists every leaf under the prefix" {
    joy init --name "Wildcards Project" --acronym TST >/dev/null
    run joy project get docs.*
    [ "$status" -eq 0 ]
    [[ "$output" == *"docs.architecture"* ]]
    [[ "$output" == *"docs.vision"* ]]
    [[ "$output" == *"docs.contributing"* ]]
}

@test "joy project get --describe adds a description to scalar values" {
    joy init --name "T" >/dev/null
    run joy project get language --describe
    [ "$status" -eq 0 ]
    [[ "$output" == *"en"* ]]
    [[ "$output" == *"project language"* ]]
}

@test "joy project get <prefix>.* --describe annotates each leaf" {
    joy init --name "T" >/dev/null
    run joy project get docs.* --describe
    [ "$status" -eq 0 ]
    [[ "$output" == *"docs.architecture"* ]]
    [[ "$output" == *"technical architecture"* ]]
    [[ "$output" == *"product-vision"* ]]
    [[ "$output" == *"contributing guide"* ]]
}

@test "joy project get * --describe lists every top-level key" {
    joy init --name "T" --acronym TST >/dev/null
    run joy project get '*' --describe
    [ "$status" -eq 0 ]
    [[ "$output" == *"name"* ]]
    [[ "$output" == *"acronym"* ]]
    [[ "$output" == *"language"* ]]
    [[ "$output" == *"docs.architecture"* ]]
}

@test "joy project get <prefix>.* --describe --json returns entries array" {
    joy init --name "T" >/dev/null
    run joy project get docs.* --describe --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.key == "docs.*"' >/dev/null
    echo "$output" | jq -e '.data.entries | type == "array"' >/dev/null
    echo "$output" | jq -e '.data.entries | map(.key) | contains(["docs.architecture", "docs.vision", "docs.contributing"])' >/dev/null
    echo "$output" | jq -e '.data.entries[] | select(.key == "docs.vision") | .description != null' >/dev/null
}

@test "joy project get key --describe --json adds description on scalar" {
    joy init --name "T" >/dev/null
    run joy project get language --describe --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.value == "en"' >/dev/null
    echo "$output" | jq -e '.data.description | type == "string"' >/dev/null
}

@test "joy project get key --json keeps the legacy {key, value} payload" {
    joy init --name "Some Name" >/dev/null
    run joy project get name --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.key == "name"' >/dev/null
    echo "$output" | jq -e '.data.value == "Some Name"' >/dev/null
    # Wildcard-only fields stay absent on the legacy form.
    [ "$(echo "$output" | jq '.data | has("entries")')" = "false" ]
    [ "$(echo "$output" | jq '.data | has("description")')" = "false" ]
}

@test "joy project get <unknown>.* exits non-zero on empty match" {
    joy init --name "T" >/dev/null
    run joy project get nonexistent.*
    [ "$status" -ne 0 ]
}

@test "joy project get description --json returns null when unset (no API break)" {
    joy init --name "T" >/dev/null
    run joy project get description --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.value == null' >/dev/null
}
