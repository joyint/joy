#!/usr/bin/env bats
# joy show: description and comments render markdown (JOY-018A-C2).

load setup

@test "joy show description preserves markdown content when piped (no ANSI)" {
    setup_human_auth
    joy add task "Test markdown" \
        --description "**Bold** text and \`code\` plus a bullet:

- first
- second" >/dev/null
    local id
    id=$(joy ls --type task --json | jq -r '.data.items[0].id')
    run joy show "$id"
    [ "$status" -eq 0 ]
    # Content survives: words from the markdown source must appear.
    [[ "$output" == *"Bold"* ]]
    [[ "$output" == *"text"* ]]
    [[ "$output" == *"code"* ]]
    [[ "$output" == *"first"* ]]
    [[ "$output" == *"second"* ]]
    # When stdout is not a TTY, output must be plain (no ANSI escape).
    ! printf '%s' "$output" | grep -q $'\x1b\\['
}

@test "joy show comment body renders markdown content without ANSI when piped" {
    setup_human_auth
    joy add task "Test comment markdown" >/dev/null
    local id
    id=$(joy ls --type task --json | jq -r '.data.items[0].id')
    joy comment "$id" "**important** point with a list:

- one
- two" >/dev/null
    run joy show "$id"
    [ "$status" -eq 0 ]
    [[ "$output" == *"important"* ]]
    [[ "$output" == *"point"* ]]
    [[ "$output" == *"one"* ]]
    [[ "$output" == *"two"* ]]
    ! printf '%s' "$output" | grep -q $'\x1b\\['
}
