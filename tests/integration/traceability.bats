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
    joy project member add dev@example.com --capabilities "implement,create"
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
