#!/usr/bin/env bats
# JOY-01BA-7D: privacy mode setting via `joy project get/set privacy` (ADR-042).
# Foundation task: the field + CLI plumbing. `anonymous` is rejected until the
# mode-transition task (JOY-01BF-2E) lands the atomic migration.

load setup

@test "joy project get privacy defaults to none on a fresh project" {
    joy init --name "T" >/dev/null
    run joy project get privacy
    [ "$status" -eq 0 ]
    [ "$output" = "none" ]
}

@test "a fresh project.yaml carries no privacy line (none is the implicit default)" {
    joy init --name "T" >/dev/null
    run grep -q "privacy" .joy/project.yaml
    [ "$status" -ne 0 ]
}

@test "joy project get privacy --json returns none by default" {
    joy init --name "T" >/dev/null
    run joy project get privacy --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.key == "privacy"' >/dev/null
    echo "$output" | jq -e '.data.value == "none"' >/dev/null
}

@test "joy project set privacy open is stored explicitly and read back" {
    joy init --name "T" >/dev/null
    run joy project set privacy open
    [ "$status" -eq 0 ]
    [[ "$output" == *"privacy = open"* ]]

    run joy project get privacy
    [ "$output" = "open" ]

    run grep -q "^privacy: open" .joy/project.yaml
    [ "$status" -eq 0 ]
}

@test "joy project set privacy none clears the field back to the default" {
    joy init --name "T" >/dev/null
    joy project set privacy open >/dev/null

    run joy project set privacy none
    [ "$status" -eq 0 ]

    run joy project get privacy
    [ "$output" = "none" ]

    # The explicit line is removed again; absent == none == open behaviour.
    run grep -q "privacy" .joy/project.yaml
    [ "$status" -ne 0 ]
}

@test "joy project set privacy anonymous requires authentication" {
    joy init --name "T" >/dev/null
    # Switching to anonymous rekeys the member map and wraps the members.yaml
    # zone key with the operator's unlocked seed; without `joy auth init` there
    # is no identity to do that, so the migration is refused.
    run joy project set privacy anonymous
    [ "$status" -ne 0 ]
    [[ "$output" == *"has no identity"* ]]

    # Nothing was written; the project stays at the default.
    run joy project get privacy
    [ "$output" = "none" ]
}

@test "joy project set privacy rejects an unknown value" {
    joy init --name "T" >/dev/null
    run joy project set privacy bogus
    [ "$status" -ne 0 ]
    [[ "$output" == *"invalid privacy mode"* ]]
}

@test "joy project get privacy --describe annotates the value" {
    joy init --name "T" >/dev/null
    run joy project get privacy --describe
    [ "$status" -eq 0 ]
    [[ "$output" == *"privacy mode"* ]]
}

@test "joy project get '*' includes privacy" {
    joy init --name "T" >/dev/null
    run joy project get '*'
    [ "$status" -eq 0 ]
    [[ "$output" == *"privacy"* ]]
}

@test "joy project overview shows the privacy mode" {
    joy init --name "T" >/dev/null
    run joy project
    [ "$status" -eq 0 ]
    [[ "$output" == *"Privacy:"* ]]
    # Effective default is open when unset.
    [[ "$output" == *"open"* ]]
}

@test "joy project set privacy requires the manage capability" {
    # setup_ai_session registers the AI member with default capabilities, which
    # exclude manage. An authenticated non-manage member must be denied.
    setup_human_auth
    setup_ai_session ai:test@joy
    run joy project set privacy open
    [ "$status" -ne 0 ]
    [[ "$output" == *"cannot perform manage"* ]]

    # And the change must not have happened.
    switch_to_human
    run joy project get privacy
    [ "$output" = "none" ]
}
