#!/usr/bin/env bats
# joy ai tutorial: renders the AI operational guide.

load setup

@test "joy ai tutorial prints the AI tutorial" {
    run joy ai tutorial
    [ "$status" -eq 0 ]
    [[ "$output" == *"Joy AI Tutorial"* ]]
    # Headline sections that must appear.
    [[ "$output" == *"Session start"* ]]
    [[ "$output" == *"Authentication"* ]]
    [[ "$output" == *"Capabilities and gates"* ]]
    [[ "$output" == *"Workflow"* ]]
    [[ "$output" == *"Commit messages"* ]]
    [[ "$output" == *"Minimum AI hygiene"* ]]
    # Token redemption pickup pattern must be documented.
    [[ "$output" == *"joy auth --token"* ]]
    [[ "$output" == *"session_env"* ]]
    [[ "$output" == *"Delegated-By"* ]]
}

@test "joy --help footer points AI tools at joy ai tutorial" {
    run joy --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"joy ai tutorial"* ]]
}
