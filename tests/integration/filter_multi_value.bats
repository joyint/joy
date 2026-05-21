#!/usr/bin/env bats
# joy ls: multi-value filter flags for --status, --type, --priority (JOY-0188-37).

load setup

@test "joy ls --status A,B keeps items with either status" {
    setup_human_auth
    joy add task "open task" >/dev/null
    joy add task "in-progress task" >/dev/null
    joy add task "review task" >/dev/null
    joy add task "closed task" >/dev/null

    # Move items to distinct statuses. joy add lands in `new`, then transitions:
    # new -> open -> in-progress -> review -> closed.
    # Portable array read: macOS ships bash 3.2, which lacks `mapfile`.
    local ids=()
    while IFS= read -r line; do ids+=("$line"); done \
        < <(joy ls --type task --all --json | jq -r '.data.items[].id')
    [ "${#ids[@]}" -eq 4 ]
    # ids[0] is the most recently added.
    joy status "${ids[3]}" open >/dev/null
    joy status "${ids[2]}" open >/dev/null && joy start "${ids[2]}" >/dev/null
    joy status "${ids[1]}" open >/dev/null && joy start "${ids[1]}" >/dev/null && joy submit "${ids[1]}" >/dev/null
    joy status "${ids[0]}" open >/dev/null && joy start "${ids[0]}" >/dev/null && joy submit "${ids[0]}" >/dev/null && joy close "${ids[0]}" >/dev/null

    run joy ls --type task --status review,closed --all --json
    [ "$status" -eq 0 ]
    local count
    count=$(echo "$output" | jq -r '.data.items | length')
    [ "$count" -eq 2 ]
}

@test "joy ls --type A,B keeps items with either type" {
    setup_human_auth
    joy add task "a task" >/dev/null
    joy add bug "a bug" >/dev/null
    joy add story "a story" >/dev/null

    run joy ls --type bug,task --json
    [ "$status" -eq 0 ]
    local count
    count=$(echo "$output" | jq -r '.data.items | length')
    [ "$count" -eq 2 ]
}

@test "joy ls --priority A,B keeps items with either priority" {
    setup_human_auth
    joy add task "low" --priority low >/dev/null
    joy add task "medium" --priority medium >/dev/null
    joy add task "high" --priority high >/dev/null

    run joy ls --priority low,high --json
    [ "$status" -eq 0 ]
    local count
    count=$(echo "$output" | jq -r '.data.items | length')
    [ "$count" -eq 2 ]
}

@test "joy ls single-value forms still work" {
    setup_human_auth
    joy add bug "a bug" >/dev/null
    joy add task "a task" >/dev/null

    run joy ls --type bug --json
    [ "$status" -eq 0 ]
    local count
    count=$(echo "$output" | jq -r '.data.items | length')
    [ "$count" -eq 1 ]
}
