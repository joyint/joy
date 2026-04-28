#!/usr/bin/env bats
# Verify that short-form item ID references in `parent` and `deps`
# are normalized at load time so that hierarchy and dependency views
# work regardless of whether the user typed the short or full form.

load setup

@test "joy ls --tree shows child under parent when parent was set via short-form ID" {
    joy init --name "Test" --acronym TST
    joy add task "Parent A"
    PARENT_FULL=$(joy ls 2>/dev/null | grep "Parent A" | awk '{print $1}')
    PARENT_SHORT="${PARENT_FULL%-*}"

    joy add task "Child B" --parent "$PARENT_SHORT"

    run joy ls --tree
    [ "$status" -eq 0 ]

    # Child must appear indented under Parent (tree connector before child)
    echo "$output" | grep -q "Parent A"
    echo "$output" | grep -E "(└──|├──).*Child B" >/dev/null
}

@test "joy deps resolves dependencies added by short-form ID" {
    joy init --name "Test" --acronym TST
    joy add task "Dep A"
    DEP_FULL=$(joy ls 2>/dev/null | grep "Dep A" | awk '{print $1}')
    DEP_SHORT="${DEP_FULL%-*}"

    joy add task "Consumer B"
    CONSUMER_FULL=$(joy ls 2>/dev/null | grep "Consumer B" | awk '{print $1}')

    joy deps "$CONSUMER_FULL" --add "$DEP_SHORT"

    run joy deps "$CONSUMER_FULL"
    [ "$status" -eq 0 ]
    [[ "$output" != *"(not found)"* ]]
    [[ "$output" == *"Dep A"* ]]
}

@test "joy ls --blocked detects items whose deps were added by short-form ID" {
    joy init --name "Test" --acronym TST
    joy add task "Blocker"
    BLOCKER_FULL=$(joy ls 2>/dev/null | grep "Blocker" | awk '{print $1}')
    BLOCKER_SHORT="${BLOCKER_FULL%-*}"

    joy add task "Blocked"
    BLOCKED_FULL=$(joy ls 2>/dev/null | grep "Blocked" | awk '{print $1}')
    joy deps "$BLOCKED_FULL" --add "$BLOCKER_SHORT"

    run joy ls --blocked
    [ "$status" -eq 0 ]
    [[ "$output" == *"Blocked"* ]]
}

@test "short-form refs are persisted in full form on next write" {
    joy init --name "Test" --acronym TST
    joy add task "Parent A"
    PARENT_FULL=$(joy ls 2>/dev/null | grep "Parent A" | awk '{print $1}')
    PARENT_SHORT="${PARENT_FULL%-*}"

    joy add task "Child B" --parent "$PARENT_SHORT"
    CHILD_FULL=$(joy ls 2>/dev/null | grep "Child B" | awk '{print $1}')

    # Right after creation, YAML still holds the short form
    grep -q "^parent: ${PARENT_SHORT}\$" .joy/items/${CHILD_FULL}-*.yaml

    # Any subsequent edit must persist the full form
    joy edit "$CHILD_FULL" --priority high
    grep -q "^parent: ${PARENT_FULL}\$" .joy/items/${CHILD_FULL}-*.yaml
    ! grep -q "^parent: ${PARENT_SHORT}\$" .joy/items/${CHILD_FULL}-*.yaml
}

@test "full-form refs remain unchanged" {
    joy init --name "Test" --acronym TST
    joy add task "Parent A"
    PARENT_FULL=$(joy ls 2>/dev/null | grep "Parent A" | awk '{print $1}')

    joy add task "Child B" --parent "$PARENT_FULL"
    CHILD_FULL=$(joy ls 2>/dev/null | grep "Child B" | awk '{print $1}')

    grep -q "^parent: ${PARENT_FULL}\$" .joy/items/${CHILD_FULL}-*.yaml

    run joy ls --tree
    [ "$status" -eq 0 ]
    echo "$output" | grep -E "(└──|├──).*Child B" >/dev/null
}
