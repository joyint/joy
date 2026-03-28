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
