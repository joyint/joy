#!/usr/bin/env bats
# Integration tests for Guard (JOY-0086).
# Tests all guard enforcement scenarios through the CLI.

load setup

# --- Helper: set up a project with lead, developer, and AI agent ---
setup_team_project() {
    setup_human_auth
    joy project member add dev@example.com --capabilities "implement,test,create"
    joy project member add ai:test@joy --capabilities "implement,review,create"
    joy add task "Test item"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Test item" | awk '{print $1}')
}

# ============================================================
# Scenario 1: Solo project (no restrictions)
# ============================================================

@test "solo project: all operations allowed" {
    joy init --name "Solo Test"
    joy add task "Solo task"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Solo task" | awk '{print $1}')
    run joy comment "$ITEM_ID" "Solo comment"
    [ "$status" -eq 0 ]
    run joy status "$ITEM_ID" in-progress
    [ "$status" -eq 0 ]
    run joy status "$ITEM_ID" closed
    [ "$status" -eq 0 ]
}

# ============================================================
# Scenario 2: AI member blocked from closing items
# ============================================================

@test "AI member can close items without gate config" {
    setup_team_project
    setup_ai_session ai:test@joy
    joy status "$ITEM_ID" in-progress
    joy status "$ITEM_ID" review
    run joy status "$ITEM_ID" closed
    [ "$status" -eq 0 ]
}

@test "AI member can submit for review" {
    setup_team_project
    setup_ai_session ai:test@joy
    joy status "$ITEM_ID" in-progress
    run joy status "$ITEM_ID" review
    [ "$status" -eq 0 ]
}

@test "AI member can start work" {
    setup_team_project
    setup_ai_session ai:test@joy
    run joy status "$ITEM_ID" in-progress
    [ "$status" -eq 0 ]
}

@test "human lead can close items" {
    setup_team_project
    joy status "$ITEM_ID" in-progress
    joy status "$ITEM_ID" review
    run joy close "$ITEM_ID"
    [ "$status" -eq 0 ]
}

# ============================================================
# Scenario 3: AI member blocked from manage actions
# ============================================================

@test "AI member cannot add project members" {
    setup_team_project
    setup_ai_session ai:test@joy
    run joy project member add someone@example.com
    [ "$status" -ne 0 ]
    [[ "$output" == *"cannot perform manage"* ]]
}

@test "AI member cannot set project properties" {
    setup_team_project
    setup_ai_session ai:test@joy
    run joy project set description "AI edited"
    [ "$status" -ne 0 ]
    [[ "$output" == *"manage"* ]]
}

# ============================================================
# Scenario 2b: Configurable gates (JOY-0030)
# ============================================================

@test "AI blocked by allow_ai gate on review->closed" {
    setup_team_project
    # Add gate config
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
    [[ "$output" == *"gate"* ]]
    [[ "$output" == *"allow_ai"* ]]
}

@test "human not blocked by allow_ai gate" {
    setup_team_project
    cat >> .joy/project.yaml << 'EOF'
status_rules:
  review -> closed:
    allow_ai: false
EOF
    joy status "$ITEM_ID" in-progress
    joy status "$ITEM_ID" review
    run joy close "$ITEM_ID"
    [ "$status" -eq 0 ]
}

@test "AI allowed on transitions without gate config" {
    setup_team_project
    cat >> .joy/project.yaml << 'EOF'
status_rules:
  new -> open:
    allow_ai: false
EOF
    # new->open is gated, but in-progress->review is not
    setup_ai_session ai:test@joy
    joy status "$ITEM_ID" in-progress
    run joy status "$ITEM_ID" review
    [ "$status" -eq 0 ]
}

# ============================================================
# Scenario 3b: Last manager protection (JOY-008C)
# ============================================================

@test "cannot remove the last member with manage capability" {
    setup_team_project
    # test@example.com is the only member with all (=manage) capabilities
    run joy project member rm test@example.com
    [ "$status" -ne 0 ]
    [[ "$output" == *"last member with manage"* ]]
}

@test "can remove a manager when another manager exists" {
    setup_human_auth
    joy project member add backup@example.com
    # Now two members with capabilities: all
    run joy project member rm backup@example.com
    [ "$status" -eq 0 ]
}

# ============================================================
# Scenario 4: Developer without manage capability
# ============================================================

@test "developer without manage cannot add members" {
    setup_team_project
    git config user.email dev@example.com
    run joy project member add newbie@example.com
    [ "$status" -ne 0 ]
    [[ "$output" == *"manage"* ]]
    git config user.email test@example.com
}

@test "developer without manage cannot delete items" {
    setup_team_project
    git config user.email dev@example.com
    run joy rm "$ITEM_ID" --force
    [ "$status" -ne 0 ]
    [[ "$output" == *"delete"* ]]
    git config user.email test@example.com
}

# ============================================================
# Scenario 5: Developer can do allowed operations
# ============================================================

@test "developer with implement can start work" {
    setup_team_project
    git config user.email dev@example.com
    run joy status "$ITEM_ID" in-progress
    [ "$status" -eq 0 ]
    git config user.email test@example.com
}

@test "developer with create can add items" {
    setup_team_project
    git config user.email dev@example.com
    run joy add task "Dev task"
    [ "$status" -eq 0 ]
    git config user.email test@example.com
}

@test "developer with create can comment" {
    setup_team_project
    git config user.email dev@example.com
    run joy comment "$ITEM_ID" "Dev comment"
    [ "$status" -eq 0 ]
    git config user.email test@example.com
}

# ============================================================
# Scenario 6: Guard events in event log
# ============================================================

@test "denied action produces guard.denied event in log" {
    setup_team_project
    # AI trying to manage (always denied) should produce guard.denied event
    setup_ai_session ai:test@joy
    run joy project set description "AI edit"
    [ "$status" -ne 0 ]
    grep -q "guard.denied" .joy/logs/*.log
}

@test "warned action produces guard.warned event in log" {
    setup_team_project
    # Developer tries to submit for review (needs Review cap, dev lacks it)
    joy status "$ITEM_ID" in-progress
    git config user.email dev@example.com
    run joy status "$ITEM_ID" review
    [ "$status" -eq 0 ]  # Warn allows but logs
    grep -q "guard.warned" .joy/logs/*.log
    git config user.email test@example.com
}

# ============================================================
# Scenario 7: Unregistered member
# ============================================================

@test "unregistered member cannot perform actions" {
    setup_team_project
    git config user.email stranger@example.com
    run joy comment "$ITEM_ID" "Stranger comment"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not a registered project member"* ]]
    git config user.email test@example.com
}

# ============================================================
# Scenario 8: All write commands are guarded
# ============================================================

@test "joy add is guarded" {
    setup_team_project
    run joy add task "Lead task"
    [ "$status" -eq 0 ]
}

@test "joy edit is guarded" {
    setup_team_project
    run joy edit "$ITEM_ID" --priority high
    [ "$status" -eq 0 ]
}

@test "joy deps add is guarded" {
    setup_team_project
    joy add task "Dep target"
    DEP_ID=$(joy ls 2>/dev/null | grep "Dep target" | awk '{print $1}')
    run joy deps "$ITEM_ID" --add "$DEP_ID"
    [ "$status" -eq 0 ]
}

@test "joy assign is guarded" {
    setup_team_project
    run joy assign "$ITEM_ID"
    [ "$status" -eq 0 ]
}

@test "joy milestone add is guarded" {
    setup_team_project
    run joy milestone add "Test MS" --date 2026-12-01
    [ "$status" -eq 0 ]
}

@test "joy release create is guarded" {
    setup_team_project
    # Create a release (needs manage capability, lead has it)
    run joy release create patch
    # May fail for other reasons (no previous version) but NOT for guard
    [[ "$output" != *"guard denied"* ]]
}

# ============================================================
# Scenario 9: Shortcuts use guard through status with AI session
# ============================================================

@test "joy start shortcut is guarded for AI session" {
    setup_team_project
    setup_ai_session ai:test@joy
    run joy start "$ITEM_ID"
    [ "$status" -eq 0 ]
}

@test "joy submit shortcut is guarded for AI session" {
    setup_team_project
    setup_ai_session ai:test@joy
    joy start "$ITEM_ID"
    run joy submit "$ITEM_ID"
    [ "$status" -eq 0 ]
}

@test "joy close shortcut works for AI without gate config" {
    setup_team_project
    setup_ai_session ai:test@joy
    joy start "$ITEM_ID"
    joy submit "$ITEM_ID"
    run joy close "$ITEM_ID"
    [ "$status" -eq 0 ]
}
