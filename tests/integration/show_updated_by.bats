#!/usr/bin/env bats
# joy show: comment indices and updated_by attribution (JOY-018B-5F).

load setup

@test "joy show: freshly added item shows updated_by alongside created_by" {
    setup_human_auth
    joy add task "test item"
    local id
    id=$(joy ls --type task --json | jq -r '.data.items[0].id')

    run joy show "$id"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Created: "* ]]
    [[ "$output" == *"Updated: "* ]]
    [[ "$output" == *"Created: "*"by test@example.com"* ]]
    [[ "$output" == *"Updated: "*"by test@example.com"* ]]
}

@test "joy show: comment carries 1-based index header" {
    setup_human_auth
    joy add task "item with comments"
    local id
    id=$(joy ls --type task --json | jq -r '.data.items[0].id')

    joy comment "$id" "first" >/dev/null
    joy comment "$id" "second" >/dev/null

    run joy show "$id"
    [ "$status" -eq 0 ]
    [[ "$output" == *"[1] "* ]]
    [[ "$output" == *"[2] "* ]]
    # Order preserved.
    local first_idx second_idx
    first_idx=$(echo "$output" | grep -n '^\[1\] ' | head -n 1 | cut -d: -f1)
    second_idx=$(echo "$output" | grep -n '^\[2\] ' | head -n 1 | cut -d: -f1)
    [ -n "$first_idx" ]
    [ -n "$second_idx" ]
    [ "$first_idx" -lt "$second_idx" ]
}

@test "joy show: comment edit refreshes updated_by and timestamp" {
    setup_human_auth
    joy add task "item to edit"
    local id
    id=$(joy ls --type task --json | jq -r '.data.items[0].id')
    joy comment "$id" "first" >/dev/null

    # Edit the comment as a different identity to prove updated_by tracks
    # the editor, not the original author.
    joy project member add dev@example.com --passphrase "$TEST_PASSPHRASE" >/dev/null
    setup_member_auth "dev@example.com" "$DEV_PASSPHRASE"
    git config user.email "dev@example.com"
    joy comment edit "$id" 1 "edited" >/dev/null
    git config user.email "test@example.com"

    run joy show "$id"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Updated: "*"by dev@example.com"* ]]
}

@test "joy show: legacy item without updated_by renders without 'by' for Updated" {
    setup_human_auth
    # Hand-written fixture for an item created before the updated_by
    # field existed. updated_by is absent; created_by present.
    mkdir -p .joy/items
    cat > .joy/items/TP-0042-legacy.yaml <<'YAML'
id: TP-0042
title: Legacy item without updated_by
type: task
status: new
priority: medium
capabilities:
- implement
created_by: test@example.com
created: 2026-01-01T00:00:00Z
updated: 2026-01-01T00:00:00Z
description: legacy fixture
YAML

    run joy show TP-0042
    [ "$status" -eq 0 ]
    [[ "$output" == *"Created: "*"by test@example.com"* ]]
    [[ "$output" == *"Updated: 2026-01-01 "* ]]
    # No "by ..." after "Updated:" because the field is absent on disk.
    # The whole line carries Created (with "by") and Updated (without).
    ! [[ "$output" =~ Updated:[[:space:]][0-9-]+[[:space:]][0-9:]+[[:space:]]by ]]
}
