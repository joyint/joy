#!/usr/bin/env bats
# Integration tests for identity resolution (JOY-0066 epic).

load setup

@test "JOY_AUTHOR sets comment author to AI member" {
    joy init --name "Test Project"
    joy add task "Test item"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Test item" | awk '{print $1}')
    joy project member add ai:test@joy
    JOY_AUTHOR=ai:test@joy joy comment "$ITEM_ID" "AI comment"
    # Check comment author in item YAML
    grep -q "author: ai:test@joy" .joy/items/*.yaml
}

@test "JOY_AUTHOR shows delegated-by in event log" {
    joy init --name "Test Project"
    joy add task "Log test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Log test" | awk '{print $1}')
    joy project member add ai:test@joy
    JOY_AUTHOR=ai:test@joy joy comment "$ITEM_ID" "Delegated action"
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

@test "--author takes priority over JOY_AUTHOR" {
    joy init --name "Test Project"
    joy add task "Priority test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Priority test" | awk '{print $1}')
    joy project member add ai:first@joy
    joy project member add ai:second@joy
    JOY_AUTHOR=ai:first@joy joy comment "$ITEM_ID" "Override" --author ai:second@joy
    # Should use --author (ai:second@joy), not JOY_AUTHOR (ai:first@joy)
    grep -q "author: ai:second@joy" .joy/items/*.yaml
}

@test "JOY_AUTHOR rejects unregistered member" {
    joy init --name "Test Project"
    joy add task "Reject test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Reject test" | awk '{print $1}')
    run env JOY_AUTHOR=nobody@invalid.com joy comment "$ITEM_ID" "Should fail"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not a registered project member"* ]]
}

@test "warning shown when AI members exist but no override set" {
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
    run env JOY_AUTHOR=ai:test@joy joy project member add someone@example.com
    [[ "$output" == *"cannot perform manage"* ]]
}
