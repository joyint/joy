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
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    # AI trying to add a member (requires manage capability)
    # Guard blocks AI from manage even with capabilities: all
    setup_ai_session ai:test@joy
    run joy project member add someone@example.com --passphrase "$TEST_PASSPHRASE"
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
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    joy add task "Status test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Status test" | awk '{print $1}')
    setup_ai_session ai:test@joy
    run joy status "$ITEM_ID" in-progress
    [ "$status" -eq 0 ]
    grep -q "ai:test@joy delegated-by:test@example.com" .joy/logs/*.log
}

@test "AI session works on assign command" {
    setup_human_auth
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
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
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    joy add task "Delegation test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Delegation test" | awk '{print $1}')
    setup_ai_session ai:test@joy
    joy comment "$ITEM_ID" "Via session"
    grep -q "ai:test@joy delegated-by:test@example.com" .joy/logs/*.log
}

@test "no warning on read-only commands with AI members" {
    setup_human_auth
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    joy add task "Read-only test"
    # joy ls is read-only, should not warn
    run joy ls
    [ "$status" -eq 0 ]
    [[ "$output" != *"AI members"* ]]
}

@test "no warning on joy show with AI members" {
    setup_human_auth
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    joy add task "Show test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Show test" | awk '{print $1}')
    run joy show "$ITEM_ID"
    [ "$status" -eq 0 ]
    [[ "$output" != *"AI members"* ]]
}

# ============================================================
# JOY_SESSION-based identity (replaces TTY-based lookup)
# ============================================================

@test "AI session works via JOY_SESSION after eval" {
    setup_human_auth
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    joy add task "Session handle test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Session handle" | awk '{print $1}')
    setup_ai_session ai:test@joy
    run joy comment "$ITEM_ID" "Via JOY_SESSION"
    [ "$status" -eq 0 ]
    grep -q "author: ai:test@joy" .joy/items/*.yaml
}

@test "AI without JOY_SESSION is not identified as AI" {
    setup_human_auth
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    joy add task "No session test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "No session" | awk '{print $1}')
    # Authenticate AI but do NOT set JOY_SESSION
    AI_TOKEN=$(joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')
    joy auth --token "$AI_TOKEN" >/dev/null 2>&1
    # Without JOY_SESSION, falls back to human session (not AI)
    joy comment "$ITEM_ID" "As human"
    # Comment should be attributed to human, not AI
    ! grep -q "author: ai:test@joy" .joy/items/*.yaml
}

@test "AI cannot impersonate another AI member" {
    setup_human_auth
    joy project member add ai:claude@joy --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:vibe@joy --passphrase "$TEST_PASSPHRASE"
    joy add task "Impersonation test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Impersonation" | awk '{print $1}')
    # Authenticate Claude
    CLAUDE_TOKEN=$(joy auth token add ai:claude@joy --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')
    eval $(joy auth --token "$CLAUDE_TOKEN")
    CLAUDE_SESSION="$JOY_SESSION"
    # Authenticate Vibe
    VIBE_TOKEN=$(joy auth token add ai:vibe@joy --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')
    eval $(joy auth --token "$VIBE_TOKEN")
    # Use Claude's session -- should be attributed to Claude, not Vibe
    JOY_SESSION="$CLAUDE_SESSION" joy comment "$ITEM_ID" "From Claude session"
    grep -q "author: ai:claude@joy" .joy/items/*.yaml
}

@test "expired AI session rejected" {
    setup_human_auth
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    joy add task "Expiry test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Expiry test" | awk '{print $1}')
    setup_ai_session ai:test@joy
    # Expire the session by patching the file
    SESSION_FILE=$(find "$XDG_STATE_HOME/joy/sessions" -name "*.json" -newer .joy/project.yaml | head -1)
    if [ -n "$SESSION_FILE" ]; then
        sed -i 's/"expires": *"[^"]*"/"expires": "2020-01-01T00:00:00Z"/' "$SESSION_FILE"
        # Expired session should not authenticate as AI
        run joy comment "$ITEM_ID" "Should not be AI"
        # Falls back to human (who is authenticated), so succeeds but not as AI
        [ "$status" -eq 0 ]
        ! grep -q "author: ai:test@joy" .joy/items/*.yaml
    fi
}

# ============================================================
# TTY isolation for human sessions
# ============================================================

@test "human session with TTY not usable from different context" {
    setup_human_auth
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    joy add task "TTY isolation test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "TTY isolation" | awk '{print $1}')
    # Re-authenticate human inside a PTY (session gets a real TTY)
    script -qc "joy auth --passphrase '$TEST_PASSPHRASE'" /dev/null
    # Outside the PTY, human session TTY does not match -> unauthenticated
    run joy comment "$ITEM_ID" "Should fail"
    [ "$status" -ne 0 ]
    [[ "$output" == *"must authenticate"* ]]
}
