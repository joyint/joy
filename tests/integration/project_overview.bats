#!/usr/bin/env bats
# joy project (no subcommand) renders every project metadata field that
# `joy project get` / `joy config` can read, so operators don't have to
# guess what's wired in `project.yaml`.

load setup

@test "joy project lists docs.* paths in the overview" {
    joy init --name "Docs Demo" --acronym DD >/dev/null
    run joy project
    [ "$status" -eq 0 ]
    [[ "$output" == *"Docs"* ]]
    [[ "$output" == *"docs/dev/architecture/README.md"* ]]
    [[ "$output" == *"docs/dev/vision/README.md"* ]]
    [[ "$output" == *"CONTRIBUTING.md"* ]]
}

@test "joy project shows description placeholder when unset" {
    joy init --name "No Desc" --acronym ND >/dev/null
    run joy project
    [ "$status" -eq 0 ]
    [[ "$output" == *"Description:"* ]]
    [[ "$output" == *"(unset)"* ]]
}

@test "joy project still shows the explicit description when set" {
    setup_human_auth
    joy project set description "real description text" >/dev/null
    run joy project
    [ "$status" -eq 0 ]
    [[ "$output" == *"real description text"* ]]
    [[ "$output" != *"(unset)"* ]]
}

@test "joy project picks up an overridden docs.architecture path" {
    setup_human_auth
    joy project set docs.architecture "ARCHITECTURE.md" >/dev/null
    run joy project
    [ "$status" -eq 0 ]
    [[ "$output" == *"ARCHITECTURE.md"* ]]
}
