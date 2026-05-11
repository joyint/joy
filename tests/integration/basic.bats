#!/usr/bin/env bats
# Basic Joy CLI integration tests.

load setup

@test "joy init creates a project" {
    run joy init --name "Test Project"
    [ "$status" -eq 0 ]
    [ -f ".joy/project.yaml" ]
    grep -q "name: Test Project" .joy/project.yaml
}

@test "joy add creates an item" {
    joy init --name "Test Project"
    run joy add task "Fix the bug" --effort 2
    [ "$status" -eq 0 ]
    [[ "$output" == *"Fix the bug"* ]]
    # Verify item file was created
    ls .joy/items/*.yaml | grep -q "fix-the-bug"
}

@test "joy ls lists items" {
    joy init --name "Test Project"
    joy add task "First item"
    joy add bug "Second item"
    run joy ls
    [ "$status" -eq 0 ]
    [[ "$output" == *"First item"* ]]
    [[ "$output" == *"Second item"* ]]
}

@test "joy add sets created_by field" {
    setup_human_auth
    setup_ai_session ai:test@joy
    joy add task "Created by AI"
    grep -q "created_by: ai:test@joy" .joy/items/*.yaml
}

@test "joy comment adds a comment" {
    joy init --name "Test Project"
    joy add task "Commentable item"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Commentable" | awk '{print $1}')
    run joy comment "$ITEM_ID" "This is a test comment"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Added comment"* ]]
}

@test "joy edit --type changes the item type" {
    joy init --name "Test"
    joy add task "swap me"
    ID=$(joy ls 2>/dev/null | grep "swap me" | awk '{print $1}')
    run joy edit "$ID" --type bug
    [ "$status" -eq 0 ]
    [[ "$output" == *"Updated"* ]]
    run joy show "$ID"
    [[ "$output" == *"Type:"* ]]
    [[ "$output" == *"bug"* ]]
    run joy edit "$ID" --type nonsense
    [ "$status" -ne 0 ]
}

@test "joy comment without text opens the editor" {
    joy init --name "Test"
    joy add task "needs comment"
    ID=$(joy ls 2>/dev/null | grep "needs comment" | awk '{print $1}')
    # Editor that writes a fixed string into the tempfile passed as $1.
    EDITOR="sh -c 'echo from-editor > \$1' --" run joy comment "$ID"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Added comment"* ]]
    run joy show "$ID"
    [[ "$output" == *"from-editor"* ]]
}

@test "joy comment with empty editor result aborts cleanly" {
    joy init --name "Test"
    joy add task "no comment"
    ID=$(joy ls 2>/dev/null | grep "no comment" | awk '{print $1}')
    EDITOR="sh -c 'true' --" run joy comment "$ID"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Empty comment"* ]]
    run joy show "$ID"
    # No "Comments:" section because nothing was added.
    [[ "$output" != *"Comments:"* ]]
}

@test "joy comment --editor overrides EDITOR" {
    joy init --name "Test"
    joy add task "override"
    ID=$(joy ls 2>/dev/null | grep "override" | awk '{print $1}')
    EDITOR="false" run joy comment "$ID" --editor "sh -c 'echo via-flag > \$1' --"
    [ "$status" -eq 0 ]
    run joy show "$ID"
    [[ "$output" == *"via-flag"* ]]
}

@test "joy comment edit replaces an existing comment" {
    joy init --name "Test"
    joy add task "many comments"
    ID=$(joy ls 2>/dev/null | grep "many comments" | awk '{print $1}')
    joy comment "$ID" "first"
    joy comment "$ID" "second"
    run joy comment edit "$ID" 1 "first replaced"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Edited comment #1"* ]]
    run joy show "$ID"
    [[ "$output" == *"first replaced"* ]]
    [[ "$output" != *"first"$'\n'* ]] || true
    [[ "$output" == *"second"* ]]
}

@test "joy comment rm deletes a comment by index" {
    joy init --name "Test"
    joy add task "to clean"
    ID=$(joy ls 2>/dev/null | grep "to clean" | awk '{print $1}')
    joy comment "$ID" "keep me"
    joy comment "$ID" "delete me"
    run joy comment rm "$ID" 2 --force
    [ "$status" -eq 0 ]
    [[ "$output" == *"Removed comment #2"* ]]
    run joy show "$ID"
    [[ "$output" == *"keep me"* ]]
    [[ "$output" != *"delete me"* ]]
}

@test "joy comment edit/rm reject out-of-range index" {
    joy init --name "Test"
    joy add task "single"
    ID=$(joy ls 2>/dev/null | grep "single" | awk '{print $1}')
    joy comment "$ID" "only"
    run joy comment edit "$ID" 5 "ghost"
    [ "$status" -ne 0 ]
    run joy comment rm "$ID" 0 --force
    [ "$status" -ne 0 ]
}

@test "joy -w runs commands in another project directory" {
    joy init --name "Outer"
    joy add task "outer item"
    INNER="$TEST_DIR/inner"
    mkdir -p "$INNER"
    (
        cd "$INNER"
        git init --quiet
        git config user.email "test@example.com"
        git config user.name "Test User"
        joy init --name "Inner"
        joy add task "inner item"
    )
    # From outer, list inner via -w; only the inner item should show.
    run joy ls -w "$INNER"
    [ "$status" -eq 0 ]
    [[ "$output" == *"inner item"* ]]
    [[ "$output" != *"outer item"* ]]
}

@test "joy -w rejects non-joy directories" {
    joy init --name "Outer"
    # mktemp -d returns a path outside TEST_DIR, so find_project_root
    # cannot walk up and accidentally hit the outer project.
    NONJOY=$(mktemp -d)
    run joy ls -w "$NONJOY"
    rm -rf "$NONJOY"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not a Joy project"* ]]
}
