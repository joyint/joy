#!/usr/bin/env bats
# Integration tests for Guard (JOY-0086).
# Tests all guard enforcement scenarios through the CLI.

load setup

# --- Helper: set up a project with lead, developer, and AI agent ---
setup_team_project() {
    joy init --name "Guard Test"
    # Lead: all capabilities (created by init, is git user test@example.com)
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
    joy status "$ITEM_ID" in-progress --author ai:test@joy
    joy status "$ITEM_ID" review --author ai:test@joy
    run joy status "$ITEM_ID" closed --author ai:test@joy
    [ "$status" -eq 0 ]
}

@test "AI member can submit for review" {
    setup_team_project
    joy status "$ITEM_ID" in-progress --author ai:test@joy
    run joy status "$ITEM_ID" review --author ai:test@joy
    [ "$status" -eq 0 ]
}

@test "AI member can start work" {
    setup_team_project
    run joy status "$ITEM_ID" in-progress --author ai:test@joy
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
    run joy project --author ai:test@joy member add someone@example.com
    [ "$status" -ne 0 ]
    [[ "$output" == *"cannot perform manage"* ]]
}

@test "AI member cannot set project properties" {
    setup_team_project
    run joy project --author ai:test@joy set description "AI edited"
    [ "$status" -ne 0 ]
    [[ "$output" == *"manage"* ]]
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
    joy init --name "Guard Test"
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
    run joy project --author ai:test@joy set description "AI edit"
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
    run joy comment "$ITEM_ID" "Stranger comment" --author stranger@example.com
    [ "$status" -ne 0 ]
    [[ "$output" == *"not a registered project member"* ]]
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
# Scenario 9: Shortcuts use guard through status with --author
# ============================================================

@test "joy start shortcut is guarded with --author" {
    setup_team_project
    run joy start "$ITEM_ID" --author ai:test@joy
    [ "$status" -eq 0 ]
}

@test "joy submit shortcut is guarded with --author" {
    setup_team_project
    joy start "$ITEM_ID" --author ai:test@joy
    run joy submit "$ITEM_ID" --author ai:test@joy
    [ "$status" -eq 0 ]
}

@test "joy close shortcut works for AI without gate config" {
    setup_team_project
    joy start "$ITEM_ID" --author ai:test@joy
    joy submit "$ITEM_ID" --author ai:test@joy
    run joy close "$ITEM_ID" --author ai:test@joy
    [ "$status" -eq 0 ]
}
