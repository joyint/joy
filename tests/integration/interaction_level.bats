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

@test "joy project member show displays levels for AI member" {
    joy init --name "Test Project"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --capabilities conceive,plan,implement,review --passphrase "$TEST_PASSPHRASE"
    run joy project member show ai:test@joy
    [ "$status" -eq 0 ]
    [[ "$output" == *"proposing"* ]]
    [[ "$output" == *"confirmed"* ]]
    [[ "$output" == *"[default]"* ]]
}

@test "joy project member show displays levels for all-capabilities member" {
    joy init --name "Test Project"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    run joy project member show ai:test@joy
    [ "$status" -eq 0 ]
    [[ "$output" == *"conceive"* ]]
    [[ "$output" == *"proposing"* ]]
    [[ "$output" == *"implement"* ]]
    [[ "$output" == *"confirmed"* ]]
}

@test "project.yaml interaction-level section overrides defaults" {
    joy init --name "Test Project"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --capabilities implement,review --passphrase "$TEST_PASSPHRASE"

    # Override the implement level in project.yaml
    cat >> .joy/project.yaml <<EOF

interaction-level:
  implement: proposing
EOF

    run joy project member show ai:test@joy
    [ "$status" -eq 0 ]
    # implement should now be proposing [project], not confirmed [default]
    [[ "$output" == *"implement"*"proposing"*"[project]"* ]]
}

@test "member edit --interaction-level sets the member default" {
    setup_human_auth
    joy project member add ai:test@joy --capabilities implement,review --passphrase "$TEST_PASSPHRASE"

    run joy project member edit ai:test@joy --interaction-level autonomous --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    grep -q "interaction-level: autonomous" .joy/project.yaml

    run joy project member show ai:test@joy
    [ "$status" -eq 0 ]
    # Both held capabilities resolve to the member default now
    [[ "$output" == *"autonomous"*"[member]"* ]]
}

@test "member edit --interaction-level CAP=LEVEL beats the member global" {
    setup_human_auth
    joy project member add ai:test@joy --capabilities implement,review --passphrase "$TEST_PASSPHRASE"

    run joy project member edit ai:test@joy --interaction-level autonomous --interaction-level review=proposing --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]

    run joy project member show ai:test@joy
    [ "$status" -eq 0 ]
    [[ "$output" == *"implement"*"autonomous"*"[member]"* ]]
    [[ "$output" == *"review"*"proposing"*"[member]"* ]]
}

@test "max-interaction-level clamps the effective level" {
    setup_human_auth
    joy project member add ai:test@joy --capabilities test --passphrase "$TEST_PASSPHRASE"

    # Set the floor via the CLI (JI-0161-C2); the command re-signs the
    # member's attestation over the new fields.
    run joy project member edit ai:test@joy --max-interaction-level test=confirmed --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]

    run joy project member show ai:test@joy
    [ "$status" -eq 0 ]
    # Default for test is autonomous, but the floor demands confirmed
    # (more oversight), so it gets clamped up to confirmed
    [[ "$output" == *"confirmed"*"[project max]"* ]]
}

@test "member edit --max-interaction-level on an unheld capability fails" {
    setup_human_auth
    joy project member add ai:test@joy --capabilities implement --passphrase "$TEST_PASSPHRASE"

    run joy project member edit ai:test@joy --max-interaction-level review=proposing --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"does not have capability"* ]]
}

@test "member edit --capabilities replaces the set and re-signs" {
    setup_human_auth
    joy project member add ai:test@joy --capabilities implement,review --passphrase "$TEST_PASSPHRASE"

    run joy project member edit ai:test@joy --capabilities plan,implement --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]

    run joy project member show ai:test@joy
    [ "$status" -eq 0 ]
    # plan is now held (shows a level), review is dropped (shows the deny mark)
    [[ "$output" == *"plan"* ]]
    [[ "$output" == *"review"*"-"* ]]

    # The re-signed attestation still covers the new capability set: the
    # signed_fields block in project.yaml now lists plan, not review.
    run grep -A20 "signed_fields:" .joy/project.yaml
    [[ "$output" == *"plan"* ]]
}

@test "member edit --add-capability and --rm-capability are incremental" {
    setup_human_auth
    joy project member add ai:test@joy --capabilities implement --passphrase "$TEST_PASSPHRASE"

    run joy project member edit ai:test@joy --add-capability review --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    run joy project member edit ai:test@joy --rm-capability implement --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]

    run joy project member show ai:test@joy
    [ "$status" -eq 0 ]
    [[ "$output" == *"review"* ]]
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
    grep -q "interaction-level:" .joy/project.yaml
    grep -q "implement: confirmed" .joy/project.yaml
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
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
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
