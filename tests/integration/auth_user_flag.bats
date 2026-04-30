#!/usr/bin/env bats
# joy auth --user <id> overrides the git-email member-selector (JOY-00F3-AE).

load setup

@test "joy auth --user authenticates as a member that differs from git email" {
    # Project member is "alice@team.com"; git config user.email is "test@example.com".
    cd "$TEST_DIR"
    joy init --name "Test" --acronym TST --user alice@team.com
    # auth init under that explicit user
    run joy auth init --user alice@team.com --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    # Drop the session, then re-authenticate using --user
    rm -rf "$XDG_STATE_HOME/joy/sessions"
    run joy auth --user alice@team.com --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"alice@team.com"* ]]
}

@test "joy auth without --user falls back to git email" {
    setup_human_auth
    rm -rf "$XDG_STATE_HOME/joy/sessions"
    run joy auth --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"test@example.com"* ]]
}

@test "joy auth --help advertises --user as a global flag" {
    cd "$TEST_DIR"
    joy init --name "Test" --acronym TST
    run joy auth --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"--user"* ]]
}
