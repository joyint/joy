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
    # Decision binding rule lives on the validity axis (JOY-01B7-0A).
    [[ "$output" == *"validity"* ]]
    [[ "$output" == *"accepted"* ]]
    # Authoring validity is documented (JOY-01B8-FA).
    [[ "$output" == *"--replaced-by"* ]]
    # What a delegation session may do at a forge (JOY-029D-3E).
    [[ "$output" == *"Forge contacts"* ]]
    [[ "$output" == *"joy forge login"* ]]
    [[ "$output" == *"delegation session"* ]]
    # The stable state words an agent reads off a failed contact.
    [[ "$output" == *"needs_sign_in"* ]]
    [[ "$output" == *"needs_host_trust"* ]]
    [[ "$output" == *"rate_limited"* ]]
    [[ "$output" == *"offline"* ]]
    [[ "$output" == *"plugin_missing"* ]]
}

@test "joy --help footer points AI tools at joy ai tutorial" {
    run joy --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"joy ai tutorial"* ]]
}
