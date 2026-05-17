#!/usr/bin/env bats
# JOY-0198-BD: CLI surface for the forge: override in project.yaml.

load setup

@test "joy project set forge github stores the value" {
    setup_human_auth
    run joy project set forge github
    [ "$status" -eq 0 ]
    [[ "$output" == *"forge = github"* ]]

    run joy project get forge
    [ "$status" -eq 0 ]
    [[ "$output" == "github" ]]
}

@test "joy project set forge none stores the explicit opt-out verbatim" {
    setup_human_auth
    run joy project set forge none
    [ "$status" -eq 0 ]

    run joy project get forge
    [ "$status" -eq 0 ]
    [[ "$output" == "none" ]]

    # Visible in the YAML so governance can audit it.
    grep -q "^forge: none" .joy/project.yaml
}

@test "joy project set forge with empty value clears the field" {
    setup_human_auth
    joy project set forge github >/dev/null
    run joy project set forge ""
    [ "$status" -eq 0 ]

    # get exits non-zero on unset scalar to match existing convention
    run joy project get forge
    [ "$status" -ne 0 ]

    # No stray `forge:` line in the YAML once cleared.
    run grep -q "^forge:" .joy/project.yaml
    [ "$status" -ne 0 ]
}

@test "joy project set forge with unsupported value is rejected" {
    setup_human_auth
    run joy project set forge gitlab
    [ "$status" -ne 0 ]
    [[ "$output" == *"unsupported forge 'gitlab'"* ]]
    [[ "$output" == *"github"* ]]
    [[ "$output" == *"none"* ]]
}

@test "joy project get forge --describe annotates the value" {
    setup_human_auth
    joy project set forge github >/dev/null
    run joy project get forge --describe
    [ "$status" -eq 0 ]
    [[ "$output" == *"github"* ]]
    [[ "$output" == *"auto-detect"* ]]
}

@test "joy project get forge --json returns null when unset" {
    setup_human_auth
    run joy project get forge --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.key == "forge"' >/dev/null
    echo "$output" | jq -e '.data.value == null' >/dev/null
}

@test "joy project get forge --json returns the value when set" {
    setup_human_auth
    joy project set forge github >/dev/null
    run joy project get forge --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.value == "github"' >/dev/null
}

@test "joy project shows the forge in the overview when set" {
    setup_human_auth
    joy project set forge github >/dev/null
    run joy project
    [ "$status" -eq 0 ]
    [[ "$output" == *"Forge:"* ]]
    [[ "$output" == *"github"* ]]
}

@test "joy project hides the forge from the overview when unset" {
    setup_human_auth
    run joy project
    [ "$status" -eq 0 ]
    [[ "$output" != *"Forge:"* ]]
}

@test "joy project get '*' --describe includes forge" {
    setup_human_auth
    run joy project get '*' --describe
    [ "$status" -eq 0 ]
    [[ "$output" == *"forge"* ]]
}
