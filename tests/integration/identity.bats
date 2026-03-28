#!/usr/bin/env bats
# Integration tests for identity resolution (JOY-0066 epic).

load setup

@test "--author sets comment author to AI member" {
    joy init --name "Test Project"
    joy add task "Test item"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Test item" | awk '{print $1}')
    joy project member add ai:test@joy
    joy comment "$ITEM_ID" "AI comment" --author ai:test@joy
    # Check comment author in item YAML
    grep -q "author: ai:test@joy" .joy/items/*.yaml
}

@test "--author shows delegated-by in event log" {
    joy init --name "Test Project"
    joy add task "Log test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Log test" | awk '{print $1}')
    joy project member add ai:test@joy
    joy comment "$ITEM_ID" "Delegated action" --author ai:test@joy
    # Event log should contain delegated-by
    grep -q "ai:test@joy delegated-by:test@example.com" .joy/logs/*.log
}

@test "--author flag sets comment author" {
    joy init --name "Test Project"
    joy add task "Author flag test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Author flag" | awk '{print $1}')
    joy project member add ai:test@joy
    joy comment "$ITEM_ID" "Via flag" --author ai:test@joy
    grep -q "author: ai:test@joy" .joy/items/*.yaml
}

@test "unregistered --author rejected" {
    joy init --name "Test Project"
    joy add task "Reject test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Reject test" | awk '{print $1}')
    run joy comment "$ITEM_ID" "Should fail" --author nobody@invalid.com
    [ "$status" -ne 0 ]
    [[ "$output" == *"not a registered project member"* ]]
}

@test "warning shown when AI members exist but no --author set" {
    joy init --name "Test Project"
    joy project member add ai:test@joy
    joy add task "Warning test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Warning test" | awk '{print $1}')
    run joy comment "$ITEM_ID" "No override"
    [ "$status" -eq 0 ]
    [[ "$output" == *"AI members"* || "$stderr" == *"AI members"* ]]
}

@test "AI member blocked from manage actions" {
    joy init --name "Test Project"
    joy project member add ai:test@joy
    # AI trying to add a member (requires manage capability)
    # Guard blocks AI from manage even with capabilities: all
    run joy project --author ai:test@joy member add someone@example.com
    [ "$status" -ne 0 ]
    [[ "$output" == *"cannot perform manage"* ]]
}

@test "--author flag works on add command" {
    joy init --name "Test Project"
    joy project member add ai:test@joy
    run joy add task "Created by AI" --author ai:test@joy
    [ "$status" -eq 0 ]
    # Event log should show AI as creator with delegated-by
    grep -q "ai:test@joy delegated-by:test@example.com" .joy/logs/*.log
}

@test "--author flag works on status command" {
    joy init --name "Test Project"
    joy project member add ai:test@joy
    joy add task "Status test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Status test" | awk '{print $1}')
    run joy status "$ITEM_ID" in-progress --author ai:test@joy
    [ "$status" -eq 0 ]
    grep -q "ai:test@joy delegated-by:test@example.com" .joy/logs/*.log
}

@test "--author flag works on assign command" {
    joy init --name "Test Project"
    joy project member add ai:test@joy
    joy add task "Assign test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Assign test" | awk '{print $1}')
    run joy assign "$ITEM_ID" --author ai:test@joy
    [ "$status" -eq 0 ]
    # AI should be assigned
    grep -q "member: ai:test@joy" .joy/items/*.yaml
}

@test "--author shows delegated-by in event log on comment" {
    joy init --name "Test Project"
    joy project member add ai:test@joy
    joy add task "Delegation test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Delegation test" | awk '{print $1}')
    joy comment "$ITEM_ID" "Via flag" --author ai:test@joy
    grep -q "ai:test@joy delegated-by:test@example.com" .joy/logs/*.log
}

@test "no warning on read-only commands with AI members" {
    joy init --name "Test Project"
    joy project member add ai:test@joy
    joy add task "Read-only test"
    # joy ls is read-only, should not warn
    run joy ls
    [ "$status" -eq 0 ]
    [[ "$output" != *"AI members"* ]]
}

@test "no warning on joy show with AI members" {
    joy init --name "Test Project"
    joy project member add ai:test@joy
    joy add task "Show test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Show test" | awk '{print $1}')
    run joy show "$ITEM_ID"
    [ "$status" -eq 0 ]
    [[ "$output" != *"AI members"* ]]
}
