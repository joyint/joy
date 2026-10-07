#!/usr/bin/env bats
# Integration tests for interaction-level defaults, resolution, and display.

load setup

TEST_PASSPHRASE="correct horse battery staple extra words"

@test "joy init creates project.defaults.yaml" {
    joy init --name "Test Project"
    [ -f ".joy/project.defaults.yaml" ]
    grep -q "interaction-level:" .joy/project.defaults.yaml
    grep -q "default: proposing" .joy/project.defaults.yaml
}

@test "project.defaults.yaml contains per-capability levels" {
    joy init --name "Test Project"
    grep -q "conceive: proposing" .joy/project.defaults.yaml
    grep -q "implement: confirmed" .joy/project.defaults.yaml
    grep -q "review: proposing" .joy/project.defaults.yaml
    grep -q "test: autonomous" .joy/project.defaults.yaml
}

@test "project.defaults.yaml contains ai-defaults capabilities" {
    joy init --name "Test Project"
    grep -q "ai-defaults:" .joy/project.defaults.yaml
    grep -q "implement" .joy/project.defaults.yaml
    grep -q "review" .joy/project.defaults.yaml
}

@test "project.defaults.yaml is gitignored" {
    joy init --name "Test Project"
    grep -q "project.defaults.yaml" .gitignore
}

@test "joy config get interaction-level.default returns proposing" {
    joy init --name "Test Project"
    run joy config get interaction-level.default
    [ "$status" -eq 0 ]
    [[ "$output" == "proposing" ]]
}

@test "joy config set interaction-level.default changes the default" {
    joy init --name "Test Project"
    joy config set interaction-level.default autonomous
    run joy config get interaction-level.default
    [ "$status" -eq 0 ]
    [[ "$output" == "autonomous" ]]
}

@test "pre-2.0 level value is rejected by config set" {
    joy init --name "Test Project"
    run joy config set interaction-level.default collaborative
    [ "$status" -ne 0 ]
    [[ "$output" == *"allowed values: autonomous, confirmed, proposing"* ]]
}

@test "old agents.default.mode key is rejected" {
    joy init --name "Test Project"
    run joy config get agents.default.mode
    [ "$status" -ne 0 ]
}

# ---------------------------------------------------------------
# What an AI member may do: capabilities and one level, on two sides
# (JI-019D-46). The project's side is what a manager signed, mine is
# what I signed for my own delegation, within the project's.
# ---------------------------------------------------------------

@test "a new AI member may act autonomously in what the project gives it" {
    setup_human_auth
    joy project member add claude --passphrase "$TEST_PASSPHRASE"
    run joy project member show claude
    [ "$status" -eq 0 ]
    [[ "$output" == *"project"*"mine"*"effective"* ]]
    [[ "$output" == *"implement"*"x"* ]]
    [[ "$output" == *"level"*"autonomous"*"autonomous"* ]]
    # neither manage nor delete unless somebody says so
    [[ "$output" != *"manage"* ]]
    [[ "$output" != *"delete"* ]]
}

@test "a person is shown with capabilities and no level" {
    setup_human_auth
    run joy project member show test@example.com
    [ "$status" -eq 0 ]
    [[ "$output" == *"manage"*"x"* ]]
    [[ "$output" != *"level"* ]]
    [[ "$output" != *"effective"* ]]
}

@test "member add takes capabilities as words and a level" {
    setup_human_auth
    run joy project member add reviewer --adapter claude --model opus \
        --capabilities review create --level confirmed --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    local file
    file="$(member_file reviewer)"
    grep -q "^adapter: claude" "$file"
    grep -q "^model: opus" "$file"
    grep -q "^level: confirmed" "$file"
    grep -q "^- review" "$file"
    ! grep -q "^- implement" "$file"
    run joy project member show reviewer
    [[ "$output" == *"claude · opus"* ]]
}

@test "member add still reads capabilities with commas" {
    setup_human_auth
    joy project member add claude --capabilities "implement,create" --passphrase "$TEST_PASSPHRASE"
    grep -q "^- implement" "$(member_file claude)"
    grep -q "^- create" "$(member_file claude)"
}

@test "member edit --project changes what the project allows and signs it again" {
    setup_human_auth
    joy project member add claude --capabilities implement create --passphrase "$TEST_PASSPHRASE"
    local before
    before=$(grep "  signature: " "$(member_file claude)")

    run joy project member edit claude --project --capabilities plan review create \
        --level confirmed --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [ "$(grep "  signature: " "$(member_file claude)")" != "$before" ]
    run joy project member show claude
    [[ "$output" == *"plan"* ]]
    [[ "$output" != *"implement"* ]]
    [[ "$output" == *"level"*"confirmed"* ]]

    # and the AI acts within it: the new signature holds
    setup_ai_session claude
    run joy add task "Written under the new maximum"
    [ "$status" -eq 0 ]
}

@test "member edit without --project sets what I allow the AI member myself" {
    setup_human_auth
    joy project member add claude --capabilities implement review create --passphrase "$TEST_PASSPHRASE"
    joy auth token add claude --passphrase "$TEST_PASSPHRASE" >/dev/null

    run joy project member edit claude --capabilities review create --level proposing \
        --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    # the project's side is untouched, mine is narrower, and that is what counts for me
    run joy project member show claude
    [[ "$output" == *"implement"*"x"*"-"*"-"* ]]
    [[ "$output" == *"level"*"autonomous"*"proposing"*"proposing"* ]]
    grep -q "^- implement" "$(member_file claude)"
}

@test "what I allow an AI member cannot go beyond what the project allows" {
    setup_human_auth
    joy project member add claude --capabilities review create --level confirmed \
        --passphrase "$TEST_PASSPHRASE"
    joy auth token add claude --passphrase "$TEST_PASSPHRASE" >/dev/null

    run joy project member edit claude --capabilities implement --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"the project does not allow claude: implement"* ]]
    run joy project member edit claude --level autonomous --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"at most confirmed"* ]]
}

@test "my own grant needs a delegation first, and says how" {
    setup_human_auth
    joy project member add claude --passphrase "$TEST_PASSPHRASE"
    run joy project member edit claude --level proposing --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"joy auth token add claude"* ]]
}

@test "a token issued before I changed my grant stops working, a new one works within it" {
    setup_human_auth
    joy project member add claude --capabilities implement review create --passphrase "$TEST_PASSPHRASE"
    joy add task "For the AI"
    ITEM_ID=$(joy ls 2>/dev/null | grep "For the AI" | awk '{print $1}')
    setup_ai_session claude
    local old_session="$JOY_SESSION"
    switch_to_human

    joy project member edit claude --capabilities review create --passphrase "$TEST_PASSPHRASE"

    run env JOY_SESSION="$old_session" joy comment "$ITEM_ID" "with the old token"
    [ "$status" -ne 0 ]
    [[ "$output" == *"has changed since its token was issued"* ]]
    [[ "$output" == *"joy auth token add claude"* ]]

    setup_ai_session claude
    run joy comment "$ITEM_ID" "with the new token"
    [ "$status" -eq 0 ]
    run joy start "$ITEM_ID"
    [ "$status" -ne 0 ]
    [[ "$output" == *"claude does not have 'implement' capability"* ]]
}

@test "an AI member never holds manage" {
    setup_human_auth
    joy project member add claude --passphrase "$TEST_PASSPHRASE"
    run joy project member edit claude --project --add-capability manage --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"never holds the manage capability"* ]]
    run joy project member edit claude --project --capabilities all --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
}

@test "a level is for an AI member, not for a person" {
    setup_human_auth
    run joy project member edit test@example.com --level confirmed --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"are for an AI member"* ]]
}

@test "member edit --capabilities replaces a person's set" {
    setup_human_auth
    DEV_OTP=$(joy project member add dev@example.com --passphrase "$TEST_PASSPHRASE" | extract_otp)
    run joy project member edit dev@example.com --capabilities plan review --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    run joy project member show dev@example.com
    [[ "$output" == *"plan"*"x"* ]]
    [[ "$output" == *"implement"*"-"* ]]
}

@test "joy show displays the level when the item has an explicit override" {
    joy init --name "Test Project"
    joy add task "Test task"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Test task" | awk '{print $1}')

    # Add the interaction-level field to the item YAML (awk for BSD/GNU portability).
    for f in ".joy/items/${ITEM_ID}-"*.yaml; do
        awk '/^status:/ { print; print "interaction-level: proposing"; next } { print }' "$f" > "${f}.tmp" \
            && mv "${f}.tmp" "$f"
    done

    run joy show "$ITEM_ID"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Interaction level:"*"proposing"* ]]
}

@test "joy show does not display the level when no override set" {
    joy init --name "Test Project"
    joy add task "Test task"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Test task" | awk '{print $1}')
    run joy show "$ITEM_ID"
    [ "$status" -eq 0 ]
    [[ "$output" != *"Interaction level:"* ]]
}

@test "joy update migrates a pre-2.0 repo to interaction-level keys and values" {
    joy init --name "Test Project"
    joy add task "Legacy task"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Legacy task" | awk '{print $1}')

    # Rebuild the pre-2.0 state: old section key, five-level values, item mode.
    cat > .joy/config.yaml <<EOF
version: 1
interaction:
  default: collaborative
EOF
    cat >> .joy/project.yaml <<EOF

interaction:
  implement: supervised
EOF
    for f in ".joy/items/${ITEM_ID}-"*.yaml; do
        awk '/^status:/ { print; print "mode: pairing"; next } { print }' "$f" > "${f}.tmp" \
            && mv "${f}.tmp" "$f"
    done

    run joy update
    [ "$status" -eq 0 ]

    grep -q "interaction-level:" .joy/config.yaml
    grep -q "default: proposing" .joy/config.yaml
    ! grep -q "^interaction:" .joy/config.yaml
    members_grep -q "interaction-level:"
    members_grep -q "implement: confirmed"
    for f in ".joy/items/${ITEM_ID}-"*.yaml; do
        grep -q "interaction-level: proposing" "$f"
        ! grep -q "^mode:" "$f"
    done

    # And the migrated repo resolves cleanly.
    run joy config get interaction-level.default
    [ "$status" -eq 0 ]
    [[ "$output" == "proposing" ]]
}

@test "joy ai init syncs project.defaults.yaml" {
    joy init --name "Test Project"
    rm .joy/project.defaults.yaml
    [ ! -f ".joy/project.defaults.yaml" ]
    # ai init should recreate it (even without tools installed)
    joy ai init </dev/null 2>/dev/null || true
    [ -f ".joy/project.defaults.yaml" ]
}

@test "joy project shows hint for member levels" {
    joy init --name "Test Project"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add testai --passphrase "$TEST_PASSPHRASE"
    run joy project
    [ "$status" -eq 0 ]
    [[ "$output" == *"joy project member show"* ]]
}

@test "joy project does not show hint without AI members" {
    joy init --name "Test Project"
    run joy project
    [ "$status" -eq 0 ]
    [[ "$output" != *"joy project member show"* ]]
}
