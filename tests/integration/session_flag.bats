#!/usr/bin/env bats
# Global --session flag: alternative to JOY_SESSION env var.

load setup

# Helper: extract the `Member:` line of the active-session block, ignoring
# the delegated-sessions listing further down the output.
active_member_line() {
    echo "$1" | grep -E "^\s*Member:" | head -n 1
}

@test "joy --session authenticates as AI member equivalent to JOY_SESSION" {
    setup_human_auth
    setup_ai_session testai
    local session="$JOY_SESSION"
    unset JOY_SESSION

    run joy --session "$session" auth status
    [ "$status" -eq 0 ]
    local member_line
    member_line=$(active_member_line "$output")
    [[ "$member_line" == *"testai"* ]]
}

@test "joy --session takes precedence over JOY_SESSION env var" {
    setup_human_auth
    setup_ai_session testai
    local good_session="$JOY_SESSION"
    # Set a bogus JOY_SESSION; --session must win.
    export JOY_SESSION="joy_s_bogusvalue"

    run joy --session "$good_session" auth status
    [ "$status" -eq 0 ]
    local member_line
    member_line=$(active_member_line "$output")
    [[ "$member_line" == *"testai"* ]]
}

@test "joy without --session and without JOY_SESSION has no active AI session" {
    setup_human_auth
    setup_ai_session testai
    unset JOY_SESSION

    run joy auth status
    [ "$status" -eq 0 ]
    # The active-session block must not name an AI member; the listing of
    # delegated sessions may still mention testai.
    local member_line
    member_line=$(active_member_line "$output")
    [[ "$member_line" != *"testai"* ]]
}
