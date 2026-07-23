#!/usr/bin/env bats
# Integration tests for interaction mode defaults, resolution, and display.

load setup

TEST_PASSPHRASE="correct horse battery staple extra words"

@test "joy init creates project.defaults.yaml" {
    joy init --name "Test Project"
    [ -f ".joy/project.defaults.yaml" ]
    grep -q "interaction:" .joy/project.defaults.yaml
    grep -q "default: collaborative" .joy/project.defaults.yaml
}

@test "project.defaults.yaml contains per-capability modes" {
    joy init --name "Test Project"
    grep -q "conceive: pairing" .joy/project.defaults.yaml
    grep -q "implement: collaborative" .joy/project.defaults.yaml
    grep -q "review: interactive" .joy/project.defaults.yaml
    grep -q "test: supervised" .joy/project.defaults.yaml
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

@test "joy config get interaction.default returns collaborative" {
    joy init --name "Test Project"
    run joy config get interaction.default
    [ "$status" -eq 0 ]
    [[ "$output" == "collaborative" ]]
}

@test "joy config set interaction.default changes the default" {
    joy init --name "Test Project"
    joy config set interaction.default pairing
    run joy config get interaction.default
    [ "$status" -eq 0 ]
    [[ "$output" == "pairing" ]]
}

@test "old agents.default.mode key is rejected" {
    joy init --name "Test Project"
    run joy config get agents.default.mode
    [ "$status" -ne 0 ]
}

@test "joy project member show displays modes for AI member" {
    joy init --name "Test Project"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --capabilities conceive,plan,implement,review --passphrase "$TEST_PASSPHRASE"
    run joy project member show ai:test@joy
    [ "$status" -eq 0 ]
    [[ "$output" == *"pairing"* ]]
    [[ "$output" == *"interactive"* ]]
    [[ "$output" == *"collaborative"* ]]
    [[ "$output" == *"[default]"* ]]
}

@test "joy project member show displays modes for all-capabilities member" {
    joy init --name "Test Project"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    run joy project member show ai:test@joy
    [ "$status" -eq 0 ]
    [[ "$output" == *"conceive"* ]]
    [[ "$output" == *"pairing"* ]]
    [[ "$output" == *"implement"* ]]
    [[ "$output" == *"collaborative"* ]]
}

@test "project.yaml modes override defaults" {
    joy init --name "Test Project"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --capabilities implement,review --passphrase "$TEST_PASSPHRASE"

    # Override implement interaction level in project.yaml
    cat >> .joy/project.yaml <<EOF

interaction:
  implement: pairing
EOF

    run joy project member show ai:test@joy
    [ "$status" -eq 0 ]
    # implement should now be pairing [project], not collaborative [default]
    [[ "$output" == *"implement"*"pairing"*"[project]"* ]]
}

@test "max-interaction clamps effective interaction" {
    setup_human_auth
    joy project member add ai:test@joy --capabilities implement --passphrase "$TEST_PASSPHRASE"

    # Set the max-interaction floor via the CLI (JI-0161-C2). This replaces the old
    # manual project.yaml awk edit; the command re-signs the member's
    # attestation over the new fields.
    run joy project member edit ai:test@joy --max-interaction implement=interactive --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]

    run joy project member show ai:test@joy
    [ "$status" -eq 0 ]
    # Default for implement is collaborative, but max-interaction is interactive (more restrictive)
    # collaborative < interactive, so it gets clamped up to interactive
    [[ "$output" == *"interactive"*"[project max]"* ]]
}

@test "member edit --max-interaction on an unheld capability fails" {
    setup_human_auth
    joy project member add ai:test@joy --capabilities implement --passphrase "$TEST_PASSPHRASE"

    run joy project member edit ai:test@joy --max-interaction review=interactive --passphrase "$TEST_PASSPHRASE"
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
    # plan is now held (shows a mode), review is dropped (shows the deny mark)
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

@test "joy show displays mode when item has explicit mode override" {
    joy init --name "Test Project"
    joy add task "Test task"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Test task" | awk '{print $1}')

    # Add mode field to item YAML (awk for BSD/GNU portability).
    for f in ".joy/items/${ITEM_ID}-"*.yaml; do
        awk '/^status:/ { print; print "mode: pairing"; next } { print }' "$f" > "${f}.tmp" \
            && mv "${f}.tmp" "$f"
    done

    run joy show "$ITEM_ID"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Mode:"*"pairing"* ]]
}

@test "joy show does not display mode when no override set" {
    joy init --name "Test Project"
    joy add task "Test task"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Test task" | awk '{print $1}')
    run joy show "$ITEM_ID"
    [ "$status" -eq 0 ]
    [[ "$output" != *"Mode:"* ]]
}

@test "joy ai init syncs project.defaults.yaml" {
    joy init --name "Test Project"
    rm .joy/project.defaults.yaml
    [ ! -f ".joy/project.defaults.yaml" ]
    # ai init should recreate it (even without tools installed)
    joy ai init </dev/null 2>/dev/null || true
    [ -f ".joy/project.defaults.yaml" ]
}

@test "joy project shows hint for member modes" {
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
