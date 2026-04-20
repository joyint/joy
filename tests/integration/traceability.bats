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
    grep -q "$ITEM_ID comment.added.*Human comment.*test@example.com" .joy/logs/*.log
}

# ============================================================
# AI identity with delegated-by in event log
# ============================================================

@test "AI item.created has delegated-by in log" {
    setup_human_auth
    setup_ai_session ai:test@joy
    joy add task "AI created"
    grep -q "item.created.*AI created.*ai:test@joy delegated-by:test@example.com" .joy/logs/*.log
}

@test "AI item.status_changed has delegated-by in log" {
    setup_human_auth
    joy add task "AI status test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "AI status" | awk '{print $1}')
    setup_ai_session ai:test@joy
    joy status "$ITEM_ID" in-progress
    grep -q "$ITEM_ID item.status_changed.*ai:test@joy delegated-by:test@example.com" .joy/logs/*.log
}

@test "AI comment.added has delegated-by in log" {
    setup_human_auth
    joy add task "AI comment test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "AI comment" | awk '{print $1}')
    setup_ai_session ai:test@joy
    joy comment "$ITEM_ID" "AI said this"
    grep -q "$ITEM_ID comment.added.*AI said this.*ai:test@joy delegated-by:test@example.com" .joy/logs/*.log
}

@test "AI item.assigned has delegated-by in log" {
    setup_human_auth
    joy add task "AI assign test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "AI assign" | awk '{print $1}')
    setup_ai_session ai:test@joy
    joy assign "$ITEM_ID"
    grep -q "$ITEM_ID item.assigned.*ai:test@joy delegated-by:test@example.com" .joy/logs/*.log
}

# ============================================================
# Auth events in event log
# ============================================================

@test "auth.session_created logged for token auth" {
    setup_human_auth
    setup_ai_session ai:test@joy
    grep -q "auth.session_created.*ai:test@joy" .joy/logs/*.log
}

# ============================================================
# Guard enforcement events in event log
# ============================================================

@test "guard.denied logged for AI manage attempt" {
    setup_human_auth
    setup_ai_session ai:test@joy
    run joy project set description "AI edit"
    [ "$status" -ne 0 ]
    grep -q "guard.denied.*ai:test@joy" .joy/logs/*.log
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
    setup_ai_session ai:test@joy
    joy status "$ITEM_ID" in-progress
    joy status "$ITEM_ID" review
    run joy status "$ITEM_ID" closed
    [ "$status" -ne 0 ]
    grep -q "guard.denied.*gate.*allow_ai.*ai:test@joy" .joy/logs/*.log
}

@test "guard.warned logged for missing capability" {
    setup_human_auth
    joy project member add dev@example.com --capabilities "implement,create" --passphrase "$TEST_PASSPHRASE"
    joy add task "Warn test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Warn test" | awk '{print $1}')
    # Developer tries review transition (needs Review cap, dev lacks it)
    joy status "$ITEM_ID" in-progress
    git config user.email dev@example.com
    joy auth init --passphrase "alpha bravo charlie delta echo foxtrot"
    joy status "$ITEM_ID" review
    grep -q "guard.warned.*dev@example.com" .joy/logs/*.log
    git config user.email test@example.com
}

# ============================================================
# Multi-identity coexistence
# ============================================================

@test "three identities coexist with correct auth status" {
    setup_human_auth
    joy project member add ai:claude@joy --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:copilot@joy --passphrase "$TEST_PASSPHRASE"
    # Create and auth both AI members
    TOKEN_CLAUDE=$(joy auth token add ai:claude@joy --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')
    TOKEN_COPILOT=$(joy auth token add ai:copilot@joy --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')
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
    [[ "$output" == *"ai:claude@joy"* ]]
    # Copilot status
    run env JOY_SESSION="$SESSION_COPILOT" joy auth status
    [[ "$output" == *"ai:copilot@joy"* ]]
}

@test "three identities produce correct event log entries" {
    setup_human_auth
    joy project member add ai:claude@joy --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:copilot@joy --passphrase "$TEST_PASSPHRASE"
    TOKEN_CLAUDE=$(joy auth token add ai:claude@joy --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')
    TOKEN_COPILOT=$(joy auth token add ai:copilot@joy --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')
    eval $(joy auth --token "$TOKEN_CLAUDE")
    SESSION_CLAUDE="$JOY_SESSION"
    eval $(joy auth --token "$TOKEN_COPILOT")
    SESSION_COPILOT="$JOY_SESSION"
    # Human creates an item
    unset JOY_SESSION
    joy add task "Human task"
    # Claude creates an item
    JOY_SESSION="$SESSION_CLAUDE" joy add task "Claude task"
    # Copilot creates an item
    JOY_SESSION="$SESSION_COPILOT" joy add task "Copilot task"
    # Verify all three identities in log
    grep -q "item.created.*Human task.*test@example.com" .joy/logs/*.log
    grep -q "item.created.*Claude task.*ai:claude@joy delegated-by:test@example.com" .joy/logs/*.log
    grep -q "item.created.*Copilot task.*ai:copilot@joy delegated-by:test@example.com" .joy/logs/*.log
}

@test "AI guard enforcement uses correct identity per session" {
    setup_human_auth
    joy project member add ai:claude@joy --capabilities "implement,create" --passphrase "$TEST_PASSPHRASE"
    TOKEN_CLAUDE=$(joy auth token add ai:claude@joy --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')
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
    joy project member add ai:claude@joy --capabilities "implement,create" --passphrase "$TEST_PASSPHRASE"
    # Copilot: can review and create, but NOT implement
    joy project member add ai:copilot@joy --capabilities "review,create" --passphrase "$TEST_PASSPHRASE"
    TOKEN_CLAUDE=$(joy auth token add ai:claude@joy --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')
    TOKEN_COPILOT=$(joy auth token add ai:copilot@joy --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')
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
    grep -q "guard.warned.*ai:copilot@joy.*implement" .joy/logs/*.log
    # Claude cannot close (lacks review), Copilot can close (has review)
    JOY_SESSION="$SESSION_CLAUDE" joy status "$CLAUDE_ID" review
    JOY_SESSION="$SESSION_COPILOT" joy status "$COPILOT_ID" review
    run env JOY_SESSION="$SESSION_COPILOT" joy status "$COPILOT_ID" closed
    [ "$status" -eq 0 ]
    # Claude closing warns (lacks review)
    run env JOY_SESSION="$SESSION_CLAUDE" joy status "$CLAUDE_ID" closed
    [ "$status" -eq 0 ]
    grep -q "guard.warned.*ai:claude@joy.*review" .joy/logs/*.log
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
    grep -q "milestone.created.*Test MS.*test@example.com" .joy/logs/*.log
}
