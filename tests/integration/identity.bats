#!/usr/bin/env bats
# Integration tests for identity resolution (JOY-0066 epic).

load setup

@test "AI session sets comment author to AI member" {
    setup_human_auth
    joy add task "Test item"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Test item" | awk '{print $1}')
    setup_ai_session ai:test@joy
    joy comment "$ITEM_ID" "AI comment"
    # Check comment author in item YAML
    grep -q "author: ai:test@joy" .joy/items/*.yaml
}

@test "AI session shows delegated-by in event log" {
    setup_human_auth
    joy add task "Log test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Log test" | awk '{print $1}')
    setup_ai_session ai:test@joy
    joy comment "$ITEM_ID" "Delegated action"
    # Event log should contain delegated-by
    grep -q "ai:test@joy delegated-by:test@example.com" .joy/logs/*.log
}

@test "AI session sets comment author" {
    setup_human_auth
    joy add task "Author flag test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Author flag" | awk '{print $1}')
    setup_ai_session ai:test@joy
    joy comment "$ITEM_ID" "Via session"
    grep -q "author: ai:test@joy" .joy/items/*.yaml
}

@test "unregistered member rejected" {
    setup_human_auth
    joy add task "Reject test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Reject test" | awk '{print $1}')
    git config user.email nobody@invalid.com
    run joy comment "$ITEM_ID" "Should fail"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not a registered project member"* ]]
    git config user.email test@example.com
}

@test "AI member blocked from manage actions" {
    setup_human_auth
    joy project member add ai:test@joy
    # AI trying to add a member (requires manage capability)
    # Guard blocks AI from manage even with capabilities: all
    setup_ai_session ai:test@joy
    run joy project member add someone@example.com
    [ "$status" -ne 0 ]
    [[ "$output" == *"cannot perform manage"* ]]
}

@test "AI session works on add command" {
    setup_human_auth
    setup_ai_session ai:test@joy
    run joy add task "Created by AI"
    [ "$status" -eq 0 ]
    # Event log should show AI as creator with delegated-by
    grep -q "ai:test@joy delegated-by:test@example.com" .joy/logs/*.log
}

@test "AI session works on status command" {
    setup_human_auth
    joy project member add ai:test@joy
    joy add task "Status test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Status test" | awk '{print $1}')
    setup_ai_session ai:test@joy
    run joy status "$ITEM_ID" in-progress
    [ "$status" -eq 0 ]
    grep -q "ai:test@joy delegated-by:test@example.com" .joy/logs/*.log
}

@test "AI session works on assign command" {
    setup_human_auth
    joy project member add ai:test@joy
    joy add task "Assign test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Assign test" | awk '{print $1}')
    setup_ai_session ai:test@joy
    run joy assign "$ITEM_ID"
    [ "$status" -eq 0 ]
    # AI should be assigned
    grep -q "member: ai:test@joy" .joy/items/*.yaml
}

@test "AI session shows delegated-by in event log on comment" {
    setup_human_auth
    joy project member add ai:test@joy
    joy add task "Delegation test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Delegation test" | awk '{print $1}')
    setup_ai_session ai:test@joy
    joy comment "$ITEM_ID" "Via session"
    grep -q "ai:test@joy delegated-by:test@example.com" .joy/logs/*.log
}

@test "no warning on read-only commands with AI members" {
    setup_human_auth
    joy project member add ai:test@joy
    joy add task "Read-only test"
    # joy ls is read-only, should not warn
    run joy ls
    [ "$status" -eq 0 ]
    [[ "$output" != *"AI members"* ]]
}

@test "no warning on joy show with AI members" {
    setup_human_auth
    joy project member add ai:test@joy
    joy add task "Show test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Show test" | awk '{print $1}')
    run joy show "$ITEM_ID"
    [ "$status" -eq 0 ]
    [[ "$output" != *"AI members"* ]]
}

# ============================================================
# AI session TTY-based lookup (JOY-0094)
# ============================================================

@test "AI session works without JOY_TOKEN after joy auth --token" {
    setup_human_auth
    joy project member add ai:test@joy
    joy add task "TTY lookup test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "TTY lookup" | awk '{print $1}')
    # Create token and authenticate (creates AI session)
    AI_TOKEN=$(joy auth create-token ai:test@joy --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')
    joy auth --token "$AI_TOKEN"
    # Remove human session so AI session is found by context match
    # (in real usage, different TTY achieves this; in tests both have no TTY)
    unset JOY_TOKEN
    joy deauth
    run joy comment "$ITEM_ID" "Via context-bound session"
    [ "$status" -eq 0 ]
    # Verify it was attributed to the AI member
    grep -q "author: ai:test@joy" .joy/items/*.yaml
}

@test "AI session via context-lookup shows delegated-by in log" {
    setup_human_auth
    joy project member add ai:test@joy
    joy add task "Delegation test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Delegation test" | awk '{print $1}')
    AI_TOKEN=$(joy auth create-token ai:test@joy --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')
    joy auth --token "$AI_TOKEN"
    unset JOY_TOKEN
    joy deauth
    joy comment "$ITEM_ID" "Context delegation"
    # Check that the comment event (not just the auth event) has delegated-by
    grep -q "comment.added.*ai:test@joy delegated-by:test@example.com" .joy/logs/*.log
}

# ============================================================
# TTY isolation (JOY-0094)
# ============================================================

@test "human session with TTY not usable from different context" {
    setup_human_auth
    joy project member add ai:test@joy
    joy add task "TTY isolation test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "TTY isolation" | awk '{print $1}')
    # Re-authenticate human inside a PTY (session gets a real TTY)
    script -qc "joy auth --passphrase '$TEST_PASSPHRASE'" /dev/null
    # Outside the PTY, human session TTY does not match -> unauthenticated
    run joy comment "$ITEM_ID" "Should fail"
    [ "$status" -ne 0 ]
    [[ "$output" == *"must authenticate"* ]]
}

@test "AI authenticates once then works freely in same context" {
    setup_human_auth
    joy project member add ai:test@joy
    joy add task "Auth once test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Auth once" | awk '{print $1}')
    # Create token while human is still authenticated (no TTY context)
    AI_TOKEN=$(joy auth create-token ai:test@joy --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')
    # Re-authenticate human inside a PTY so its session gets a real TTY
    script -qc "joy auth --passphrase '$TEST_PASSPHRASE'" /dev/null
    # Authenticate AI outside PTY (session with tty: None)
    joy auth --token "$AI_TOKEN"
    unset JOY_TOKEN
    # Subsequent commands without token should work via context-bound session
    # (human session has TTY, doesn't match; AI session has None, matches)
    run joy comment "$ITEM_ID" "AI working freely"
    [ "$status" -eq 0 ]
    grep -q "author: ai:test@joy" .joy/items/*.yaml
}
