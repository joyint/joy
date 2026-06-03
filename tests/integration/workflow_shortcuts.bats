#!/usr/bin/env bats
# Status shortcuts approve / defer / rework, and start staying new-tolerant
# (JOY-01AC-14). The verbs are named target-status shortcuts; restrictions
# come only from gates, so these tests assert the target status is set, not
# that any source state is rejected.

load setup

# Create an item and echo its ID.
add_task() {
    joy add task "$1" >/dev/null
    joy ls 2>/dev/null | grep "$1" | awk '{print $1}'
}

@test "joy approve moves new -> open" {
    setup_human_auth
    ID=$(add_task "Triage me")
    run joy approve "$ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.from == "new"' >/dev/null
    echo "$output" | jq -e '.data.to == "open"' >/dev/null
}

@test "joy start works directly from new (zero-ceremony, no approve needed)" {
    setup_human_auth
    ID=$(add_task "Just do it")
    run joy start "$ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.from == "new"' >/dev/null
    echo "$output" | jq -e '.data.to == "in-progress"' >/dev/null
}

@test "joy start works from open too (after approve)" {
    setup_human_auth
    ID=$(add_task "Approved then started")
    joy approve "$ID" >/dev/null
    run joy start "$ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.from == "open"' >/dev/null
    echo "$output" | jq -e '.data.to == "in-progress"' >/dev/null
}

@test "joy rework moves review -> in-progress" {
    setup_human_auth
    ID=$(add_task "Needs rework")
    joy start "$ID" >/dev/null
    joy submit "$ID" >/dev/null
    run joy rework "$ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.from == "review"' >/dev/null
    echo "$output" | jq -e '.data.to == "in-progress"' >/dev/null
}

@test "joy defer works from new" {
    setup_human_auth
    ID=$(add_task "Defer from new")
    run joy defer "$ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.from == "new"' >/dev/null
    echo "$output" | jq -e '.data.to == "deferred"' >/dev/null
}

@test "joy defer works from open" {
    setup_human_auth
    ID=$(add_task "Defer from open")
    joy approve "$ID" >/dev/null
    run joy defer "$ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.from == "open"' >/dev/null
    echo "$output" | jq -e '.data.to == "deferred"' >/dev/null
}

@test "joy defer works from in-progress" {
    setup_human_auth
    ID=$(add_task "Defer from wip")
    joy start "$ID" >/dev/null
    run joy defer "$ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.from == "in-progress"' >/dev/null
    echo "$output" | jq -e '.data.to == "deferred"' >/dev/null
}

@test "joy defer works from review" {
    setup_human_auth
    ID=$(add_task "Defer from review")
    joy start "$ID" >/dev/null
    joy submit "$ID" >/dev/null
    run joy defer "$ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.from == "review"' >/dev/null
    echo "$output" | jq -e '.data.to == "deferred"' >/dev/null
}

@test "joy approve appears in joy --help shortcuts" {
    run joy --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"approve"* ]]
    [[ "$output" == *"rework"* ]]
    [[ "$output" == *"defer"* ]]
}
