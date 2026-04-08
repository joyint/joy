#!/usr/bin/env bats
# Integration tests for interaction mode defaults, resolution, and display.

load setup

@test "joy init creates project.defaults.yaml" {
    joy init --name "Test Project"
    [ -f ".joy/project.defaults.yaml" ]
    grep -q "modes:" .joy/project.defaults.yaml
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

@test "joy config get modes.default returns collaborative" {
    joy init --name "Test Project"
    run joy config get modes.default
    [ "$status" -eq 0 ]
    [[ "$output" == "collaborative" ]]
}

@test "joy config set modes.default changes the default" {
    joy init --name "Test Project"
    joy config set modes.default pairing
    run joy config get modes.default
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
    joy project member add ai:test@joy --capabilities conceive,plan,implement,review
    run joy project member show ai:test@joy
    [ "$status" -eq 0 ]
    [[ "$output" == *"pairing"* ]]
    [[ "$output" == *"interactive"* ]]
    [[ "$output" == *"collaborative"* ]]
    [[ "$output" == *"[default]"* ]]
}

@test "joy project member show displays modes for all-capabilities member" {
    joy init --name "Test Project"
    joy project member add ai:test@joy
    run joy project member show ai:test@joy
    [ "$status" -eq 0 ]
    [[ "$output" == *"conceive"* ]]
    [[ "$output" == *"pairing"* ]]
    [[ "$output" == *"implement"* ]]
    [[ "$output" == *"collaborative"* ]]
}

@test "project.yaml modes override defaults" {
    joy init --name "Test Project"
    joy project member add ai:test@joy --capabilities implement,review

    # Override implement mode in project.yaml
    cat >> .joy/project.yaml <<EOF

modes:
  implement: pairing
EOF

    run joy project member show ai:test@joy
    [ "$status" -eq 0 ]
    # implement should now be pairing [project], not collaborative [default]
    [[ "$output" == *"implement"*"pairing"*"[project]"* ]]
}

@test "max-mode clamps effective mode" {
    setup_human_auth
    joy project member add ai:test@joy --capabilities implement

    # Set max-mode on the member's implement capability
    # We need to manually edit project.yaml for this
    sed -i 's/implement: {}/implement:\n        max-mode: interactive/' .joy/project.yaml

    run joy project member show ai:test@joy
    [ "$status" -eq 0 ]
    # Default for implement is collaborative, but max-mode is interactive (more restrictive)
    # collaborative < interactive, so it gets clamped up to interactive
    [[ "$output" == *"interactive"*"[project max]"* ]]
}

@test "joy show displays mode when item has explicit mode override" {
    joy init --name "Test Project"
    joy add task "Test task"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Test task" | awk '{print $1}')

    # Add mode field to item YAML
    sed -i '/^status:/a mode: pairing' ".joy/items/${ITEM_ID}-"*.yaml

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
    joy project member add ai:test@joy
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
