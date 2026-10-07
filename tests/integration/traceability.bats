#!/usr/bin/env bats
# Integration tests for event log traceability (JOY-008F).
# Verifies that all actions produce correct event log entries
# with proper identity attribution.

load setup

# ============================================================
# Human identity in event log
# ============================================================

@test "human item.created has correct author in log" {
    setup_human_auth
    joy add task "Human created"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Human created" | awk '{print $1}')
    grep -q "$ITEM_ID item.created.*test@example.com" .joy/logs/*.log
}

@test "human item.status_changed has correct author in log" {
    setup_human_auth
    joy add task "Status log test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Status log" | awk '{print $1}')
    joy status "$ITEM_ID" in-progress
    grep -q "$ITEM_ID item.status_changed.*new -> in-progress.*test@example.com" .joy/logs/*.log
}

@test "human comment.added has correct author in log" {
    setup_human_auth
    joy add task "Comment log test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Comment log" | awk '{print $1}')
    joy comment "$ITEM_ID" "Human comment"
    # Comment text is no longer recorded in the log (JOY-0175-9B);
    # the structural event + actor are what we verify here.
    grep -q "$ITEM_ID comment.added.*test@example.com" .joy/logs/*.log
}

# ============================================================
# AI identity with delegated-by in event log
# ============================================================

@test "AI item.created has delegated-by in log" {
    setup_human_auth
    setup_ai_session testai
    joy add task "AI created"
    # Item title is not recorded in the log (JOY-0175-9B); verify the
    # structural event and the delegated-by actor.
    grep -q "item.created .*testai delegated-by:test@example.com" .joy/logs/*.log
}

@test "AI item.status_changed has delegated-by in log" {
    setup_human_auth
    joy add task "AI status test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "AI status" | awk '{print $1}')
    setup_ai_session testai
    joy status "$ITEM_ID" in-progress
    grep -q "$ITEM_ID item.status_changed.*testai delegated-by:test@example.com" .joy/logs/*.log
}

@test "AI comment.added has delegated-by in log" {
    setup_human_auth
    joy add task "AI comment test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "AI comment" | awk '{print $1}')
    setup_ai_session testai
    joy comment "$ITEM_ID" "AI said this"
    # Comment text is not recorded in the log (JOY-0175-9B).
    grep -q "$ITEM_ID comment.added .*testai delegated-by:test@example.com" .joy/logs/*.log
}

@test "AI item.assigned has delegated-by in log" {
    setup_human_auth
    joy add task "AI assign test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "AI assign" | awk '{print $1}')
    setup_ai_session testai
    joy assign "$ITEM_ID"
    grep -q "$ITEM_ID item.assigned.*testai delegated-by:test@example.com" .joy/logs/*.log
}

# ============================================================
# Auth events in event log
# ============================================================

@test "auth.session_created logged for token auth" {
    setup_human_auth
    setup_ai_session testai
    grep -q "auth.session_created.*testai" .joy/logs/*.log
}

# ============================================================
# Guard enforcement events in event log
# ============================================================

@test "guard.denied logged for AI manage attempt" {
    setup_human_auth
    setup_ai_session testai
    run joy project set description "AI edit"
    [ "$status" -ne 0 ]
    grep -q "guard.denied.*testai" .joy/logs/*.log
}

@test "guard.denied logged for gate violation" {
    setup_human_auth
    joy add task "Gate test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Gate test" | awk '{print $1}')
    cat >> .joy/project.yaml << 'EOF'
status_rules:
  review -> closed:
    allow_ai: false
EOF
    setup_ai_session testai
    joy status "$ITEM_ID" in-progress
    joy status "$ITEM_ID" review
    run joy status "$ITEM_ID" closed
    [ "$status" -ne 0 ]
    grep -q "guard.denied.*gate.*allow_ai.*testai" .joy/logs/*.log
}

@test "guard.warned logged for missing capability" {
    setup_human_auth
    DEV_OTP=$(joy project member add dev@example.com --capabilities "implement,create" --passphrase "$TEST_PASSPHRASE" | extract_otp)
    joy add task "Warn test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Warn test" | awk '{print $1}')
    # Developer tries review transition (needs Review cap, dev lacks it)
    joy status "$ITEM_ID" in-progress
    # Dev redeems their invitation, which enrols them. Naming dev in
    # this repository's git config is what makes the bare command below
    # act as dev (JOY-02AE-1A).
    joy auth --otp "$DEV_OTP" --user dev@example.com --passphrase "alpha bravo charlie delta echo foxtrot"
    git config user.email dev@example.com
    joy status "$ITEM_ID" review
    grep -q "guard.warned.*dev@example.com" .joy/logs/*.log
}

# ============================================================
# Multi-identity coexistence
# ============================================================

@test "three identities coexist with correct auth status" {
    setup_human_auth
    joy project member add claude --passphrase "$TEST_PASSPHRASE"
    joy project member add copilot --passphrase "$TEST_PASSPHRASE"
    # Create and auth both AI members
    TOKEN_CLAUDE=$(joy auth token add claude --passphrase "$TEST_PASSPHRASE" \
        | tr -d '"')
    TOKEN_COPILOT=$(joy auth token add copilot --passphrase "$TEST_PASSPHRASE" \
        | tr -d '"')
    eval $(joy auth --token "$TOKEN_CLAUDE")
    SESSION_CLAUDE="$JOY_SESSION"
    eval $(joy auth --token "$TOKEN_COPILOT")
    SESSION_COPILOT="$JOY_SESSION"
    # Human status (no JOY_SESSION)
    unset JOY_SESSION
    run joy auth status
    [[ "$output" == *"test@example.com"* ]]
    # Claude status
    run env JOY_SESSION="$SESSION_CLAUDE" joy auth status
    [[ "$output" == *"claude"* ]]
    # Copilot status
    run env JOY_SESSION="$SESSION_COPILOT" joy auth status
    [[ "$output" == *"copilot"* ]]
}

@test "three identities produce correct event log entries" {
    setup_human_auth
    joy project member add claude --passphrase "$TEST_PASSPHRASE"
    joy project member add copilot --passphrase "$TEST_PASSPHRASE"
    TOKEN_CLAUDE=$(joy auth token add claude --passphrase "$TEST_PASSPHRASE" \
        | tr -d '"')
    TOKEN_COPILOT=$(joy auth token add copilot --passphrase "$TEST_PASSPHRASE" \
        | tr -d '"')
    eval $(joy auth --token "$TOKEN_CLAUDE")
    SESSION_CLAUDE="$JOY_SESSION"
    eval $(joy auth --token "$TOKEN_COPILOT")
    SESSION_COPILOT="$JOY_SESSION"
    # Capture IDs as items are created so we can verify the per-item
    # actor without relying on titles in the log (JOY-0175-9B).
    unset JOY_SESSION
    HUMAN_ID=$(joy add task "Human task" | sed -n 's/^Created \([A-Z0-9-]\+\) .*/\1/p')
    CLAUDE_ID=$(JOY_SESSION="$SESSION_CLAUDE" joy add task "Claude task" \
        | sed -n 's/^Created \([A-Z0-9-]\+\) .*/\1/p')
    COPILOT_ID=$(JOY_SESSION="$SESSION_COPILOT" joy add task "Copilot task" \
        | sed -n 's/^Created \([A-Z0-9-]\+\) .*/\1/p')
    grep -q "$HUMAN_ID item.created .*test@example.com" .joy/logs/*.log
    grep -q "$CLAUDE_ID item.created .*claude delegated-by:test@example.com" .joy/logs/*.log
    grep -q "$COPILOT_ID item.created .*copilot delegated-by:test@example.com" .joy/logs/*.log
}

@test "AI guard enforcement uses correct identity per session" {
    setup_human_auth
    joy project member add claude --capabilities "implement,create" --passphrase "$TEST_PASSPHRASE"
    TOKEN_CLAUDE=$(joy auth token add claude --passphrase "$TEST_PASSPHRASE" \
        | tr -d '"')
    eval $(joy auth --token "$TOKEN_CLAUDE")
    SESSION_CLAUDE="$JOY_SESSION"
    # Claude cannot manage (no manage capability)
    run env JOY_SESSION="$SESSION_CLAUDE" joy project set description "Claude edit"
    [ "$status" -ne 0 ]
    [[ "$output" == *"manage"* ]]
    # Human can manage
    unset JOY_SESSION
    run joy project set description "Human edit"
    [ "$status" -eq 0 ]
}

@test "two AIs with different capabilities enforced correctly" {
    setup_human_auth
    # Claude: can implement and create, but NOT delete
    joy project member add claude --capabilities "implement,create" --passphrase "$TEST_PASSPHRASE"
    # Copilot: can review and create, but NOT implement
    joy project member add copilot --capabilities "review,create" --passphrase "$TEST_PASSPHRASE"
    TOKEN_CLAUDE=$(joy auth token add claude --passphrase "$TEST_PASSPHRASE" \
        | tr -d '"')
    TOKEN_COPILOT=$(joy auth token add copilot --passphrase "$TEST_PASSPHRASE" \
        | tr -d '"')
    eval $(joy auth --token "$TOKEN_CLAUDE")
    SESSION_CLAUDE="$JOY_SESSION"
    eval $(joy auth --token "$TOKEN_COPILOT")
    SESSION_COPILOT="$JOY_SESSION"
    # Both can create items
    JOY_SESSION="$SESSION_CLAUDE" joy add task "Claude item"
    JOY_SESSION="$SESSION_COPILOT" joy add task "Copilot item"
    CLAUDE_ID=$(joy ls 2>/dev/null | grep "Claude item" | awk '{print $1}')
    COPILOT_ID=$(joy ls 2>/dev/null | grep "Copilot item" | awk '{print $1}')
    # Claude can start work (implement), Copilot cannot (warn)
    run env JOY_SESSION="$SESSION_CLAUDE" joy status "$CLAUDE_ID" in-progress
    [ "$status" -eq 0 ]
    run env JOY_SESSION="$SESSION_COPILOT" joy status "$COPILOT_ID" in-progress
    # Copilot lacks implement -> warn (still succeeds, but warning logged)
    [ "$status" -eq 0 ]
    grep -q "guard.warned.*copilot.*implement" .joy/logs/*.log
    # Claude cannot close (lacks review), Copilot can close (has review)
    JOY_SESSION="$SESSION_CLAUDE" joy status "$CLAUDE_ID" review
    JOY_SESSION="$SESSION_COPILOT" joy status "$COPILOT_ID" review
    run env JOY_SESSION="$SESSION_COPILOT" joy status "$COPILOT_ID" closed
    [ "$status" -eq 0 ]
    # Claude closing warns (lacks review)
    run env JOY_SESSION="$SESSION_CLAUDE" joy status "$CLAUDE_ID" closed
    [ "$status" -eq 0 ]
    grep -q "guard.warned.*claude.*review" .joy/logs/*.log
}

# ============================================================
# Dep and milestone events
# ============================================================

@test "dep.added has correct author in log" {
    setup_human_auth
    joy add task "Item A"
    joy add task "Item B"
    ID_A=$(joy ls 2>/dev/null | grep "Item A" | awk '{print $1}')
    ID_B=$(joy ls 2>/dev/null | grep "Item B" | awk '{print $1}')
    joy deps "$ID_A" --add "$ID_B"
    grep -q "$ID_A dep.added.*$ID_B.*test@example.com" .joy/logs/*.log
}

@test "milestone.created has correct author in log" {
    setup_human_auth
    joy milestone add "Test MS" --date 2026-12-01
    # The title is no longer recorded in the event log (JOY-0175-9B);
    # match the structural target id and the actor instead.
    MS_ID=$(joy milestone ls 2>/dev/null | grep "Test MS" | awk '{print $1}')
    grep -q "$MS_ID milestone.created.*test@example.com" .joy/logs/*.log
}
