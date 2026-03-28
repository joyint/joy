#!/usr/bin/env bats
# Integration tests for auto-git feature (JOY-005D).

load setup

# -- auto-git: add (default) -------------------------------------------------

@test "auto-git add: joy init stages project files" {
    joy init --name "Auto Git Test"
    # project.yaml and config.defaults.yaml should be staged
    staged=$(git diff --cached --name-only)
    [[ "$staged" == *".joy/project.yaml"* ]]
    [[ "$staged" == *".joy/config.defaults.yaml"* ]]
    [[ "$staged" == *".gitignore"* ]]
}

@test "auto-git add: joy add stages item and log files" {
    joy init --name "Auto Git Test"
    git add -A && git commit -m "init [no-item]" --quiet
    joy add task "Staged item" --effort 2
    staged=$(git diff --cached --name-only)
    [[ "$staged" == *".joy/items/"* ]]
    [[ "$staged" == *".joy/logs/"* ]]
}

@test "auto-git add: joy status stages updated item" {
    joy init --name "Auto Git Test"
    joy add task "Status test"
    git add -A && git commit -m "init [no-item]" --quiet
    ITEM_ID=$(joy ls 2>/dev/null | grep "Status test" | awk '{print $1}')
    joy start "$ITEM_ID"
    staged=$(git diff --cached --name-only)
    [[ "$staged" == *".joy/items/"* ]]
}

@test "auto-git add: joy comment stages item" {
    joy init --name "Auto Git Test"
    joy add task "Comment test"
    git add -A && git commit -m "init [no-item]" --quiet
    ITEM_ID=$(joy ls 2>/dev/null | grep "Comment test" | awk '{print $1}')
    joy comment "$ITEM_ID" "A comment"
    staged=$(git diff --cached --name-only)
    [[ "$staged" == *".joy/items/"* ]]
}

@test "auto-git add: joy edit stages item" {
    joy init --name "Auto Git Test"
    joy add task "Edit test"
    git add -A && git commit -m "init [no-item]" --quiet
    ITEM_ID=$(joy ls 2>/dev/null | grep "Edit test" | awk '{print $1}')
    joy edit "$ITEM_ID" --priority high
    staged=$(git diff --cached --name-only)
    [[ "$staged" == *".joy/items/"* ]]
}

@test "auto-git add: joy rm stages deleted item" {
    joy init --name "Auto Git Test"
    joy add task "Delete me"
    git add -A && git commit -m "init [no-item]" --quiet
    ITEM_ID=$(joy ls 2>/dev/null | grep "Delete me" | awk '{print $1}')
    joy rm "$ITEM_ID" --force
    staged=$(git diff --cached --name-only)
    [[ "$staged" == *".joy/items/"* ]]
}

@test "auto-git add: milestone add stages milestone file" {
    joy init --name "Auto Git Test"
    git add -A && git commit -m "init [no-item]" --quiet
    joy milestone add "Beta Release"
    staged=$(git diff --cached --name-only)
    [[ "$staged" == *".joy/milestones/"* ]]
}

# -- auto-git: off ------------------------------------------------------------

@test "auto-git off: nothing is staged after joy add" {
    joy init --name "Auto Git Test"
    git add -A && git commit -m "init [no-item]" --quiet
    joy config set workflow.auto-git off
    joy add task "Not staged"
    staged=$(git diff --cached --name-only)
    [ -z "$staged" ]
}

# -- auto-git: commit ---------------------------------------------------------

@test "auto-git commit: joy add creates a commit" {
    joy init --name "Auto Git Test"
    git add -A && git commit -m "init [no-item]" --quiet
    joy config set workflow.auto-git commit
    joy add task "Auto committed" --effort 3
    # The latest commit should be from joy
    last_msg=$(git log -1 --format=%s)
    [[ "$last_msg" == joy:* ]]
    [[ "$last_msg" == *"Auto committed"* ]]
}

@test "auto-git commit: commit message contains Co-Authored-By" {
    joy init --name "Auto Git Test"
    git add -A && git commit -m "init [no-item]" --quiet
    joy config set workflow.auto-git commit
    joy add task "With trailer"
    last_body=$(git log -1 --format=%b)
    [[ "$last_body" == *"Co-Authored-By:"* ]]
    [[ "$last_body" == *"test@example.com"* ]]
}

@test "auto-git commit: status change creates commit with details" {
    joy init --name "Auto Git Test"
    git add -A && git commit -m "init [no-item]" --quiet
    joy config set workflow.auto-git commit
    joy add task "Status commit"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Status commit" | awk '{print $1}')
    joy start "$ITEM_ID"
    last_msg=$(git log -1 --format=%s)
    [[ "$last_msg" == *"-> in-progress"* ]]
}

@test "auto-git commit: AI identity in Co-Authored-By" {
    joy init --name "Auto Git Test"
    git add -A && git commit -m "init [no-item]" --quiet
    joy config set workflow.auto-git commit
    joy project member add ai:test@joy
    joy add task "AI commit" --author ai:test@joy
    last_body=$(git log -1 --format=%b)
    [[ "$last_body" == *"Co-Authored-By: ai:test@joy"* ]]
}

@test "auto-git commit: no commit when nothing changed" {
    joy init --name "Auto Git Test"
    git add -A && git commit -m "init [no-item]" --quiet
    joy config set workflow.auto-git commit
    # joy ls is read-only, should not create a commit
    before=$(git rev-parse HEAD)
    joy ls 2>/dev/null
    after=$(git rev-parse HEAD)
    [ "$before" = "$after" ]
}

# -- auto-git: edit title renames file -----------------------------------------

@test "auto-git add: title rename stages both old and new file" {
    joy init --name "Auto Git Test"
    joy add task "Old title"
    git add -A && git commit -m "init [no-item]" --quiet
    ITEM_ID=$(joy ls 2>/dev/null | grep "Old title" | awk '{print $1}')
    joy edit "$ITEM_ID" --title "New title"
    staged=$(git diff --cached --name-only)
    # New file should be staged
    [[ "$staged" == *"new-title"* ]]
}
