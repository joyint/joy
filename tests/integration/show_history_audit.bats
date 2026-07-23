#!/usr/bin/env bats
# joy show: item attribute history footer + per-comment edit list (JOY-018F-A4).

load setup

@test "joy show: footer carries Created on its own line for a fresh item" {
    setup_human_auth
    joy add task "fresh item"
    local id
    id=$(joy ls --type task --json | jq -r '.data.items[0].id')

    run joy show "$id"
    [ "$status" -eq 0 ]
    # Created line is its own line, with `by`.
    echo "$output" | grep -E "^Created: [0-9-]+ [0-9:]+ by test@example\.com$"
    # Fresh items have no attribute updates yet: no Updated line in the footer.
    ! echo "$output" | grep -E "^Updated: "
}

@test "joy show: attribute mutations append Updated lines to the footer" {
    setup_human_auth
    joy add task "mutated item"
    local id
    id=$(joy ls --type task --json | jq -r '.data.items[0].id')

    joy edit "$id" --priority high >/dev/null
    joy edit "$id" --priority critical >/dev/null

    run joy show "$id"
    [ "$status" -eq 0 ]
    local updated_count
    updated_count=$(echo "$output" | grep -cE "^Updated: ")
    [ "$updated_count" -eq 2 ]
}

@test "joy show: comment add/edit/rm do NOT add to item footer history" {
    setup_human_auth
    joy add task "commented item"
    local id
    id=$(joy ls --type task --json | jq -r '.data.items[0].id')

    joy comment "$id" "first" >/dev/null
    joy comment edit "$id" 1 "edited" >/dev/null
    joy comment "$id" "second" >/dev/null

    run joy show "$id"
    [ "$status" -eq 0 ]
    # No attribute mutations happened, so no Updated entries should be in the footer.
    ! echo "$output" | grep -E "^Updated: "
}

@test "joy show: comment edit preserves original author and date, records editor in updates block" {
    setup_human_auth
    joy add task "comment edit audit"
    local id
    id=$(joy ls --type task --json | jq -r '.data.items[0].id')
    joy comment "$id" "original body" >/dev/null

    # Edit as a different identity.
    DEV_OTP=$(joy project member add dev@example.com --passphrase "$TEST_PASSPHRASE" | extract_otp)
    setup_member_auth "dev@example.com" "$DEV_PASSPHRASE"
    git config user.email "dev@example.com"
    joy comment edit "$id" 1 "edited body" >/dev/null
    git config user.email "test@example.com"

    run joy show "$id"
    [ "$status" -eq 0 ]
    # Comment header still shows the original author (test@example.com).
    [[ "$output" == *"[1] "*"by test@example.com"* ]]
    # The new body replaces the old one.
    [[ "$output" == *"edited body"* ]]
    [[ "$output" != *"original body"* ]]
    # An indented `Updated:` line names the editor (dev@example.com).
    echo "$output" | grep -E "^  Updated: [0-9-]+ [0-9:]+ by dev@example\.com$"
}

@test "joy show: legacy item without history still renders single Updated line" {
    setup_human_auth
    mkdir -p .joy/items
    cat > .joy/items/TP-0099-legacy.yaml <<'YAML'
id: TP-0099
title: Legacy item before history
type: task
status: in-progress
priority: medium
capabilities:
- implement
created_by: alice@example.com
created: 2026-01-01T09:00:00Z
updated: 2026-01-02T10:30:00Z
updated_by: bob@example.com
description: legacy fixture
YAML

    run joy show TP-0099
    [ "$status" -eq 0 ]
    echo "$output" | grep -E "^Created: 2026-01-01 [0-9:]+ by alice@example\.com$"
    echo "$output" | grep -E "^Updated: 2026-01-02 [0-9:]+ by bob@example\.com$"
}

@test "joy show: legacy item with updated == created shows no Updated line" {
    setup_human_auth
    mkdir -p .joy/items
    cat > .joy/items/TP-0100-fresh.yaml <<'YAML'
id: TP-0100
title: Legacy item never mutated
type: task
status: new
priority: medium
capabilities:
- implement
created_by: alice@example.com
created: 2026-01-01T09:00:00Z
updated: 2026-01-01T09:00:00Z
description: legacy fixture
YAML

    run joy show TP-0100
    [ "$status" -eq 0 ]
    echo "$output" | grep -E "^Created: "
    ! echo "$output" | grep -E "^Updated: "
}
