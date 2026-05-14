#!/usr/bin/env bats
# Global --session flag: alternative to JOY_SESSION env var.

load setup

@test "joy --session authenticates as AI member equivalent to JOY_SESSION" {
    setup_human_auth
    setup_ai_session ai:test@joy
    local session="$JOY_SESSION"
    unset JOY_SESSION

    run joy --session "$session" auth status
    [ "$status" -eq 0 ]
    [[ "$output" == *"ai:test@joy"* ]]
}

@test "joy --session takes precedence over JOY_SESSION env var" {
    setup_human_auth
    setup_ai_session ai:test@joy
    local good_session="$JOY_SESSION"
    # Set a bogus JOY_SESSION; --session must win.
    export JOY_SESSION="joy_s_bogusvalue"

    run joy --session "$good_session" auth status
    [ "$status" -eq 0 ]
    [[ "$output" == *"ai:test@joy"* ]]
}

@test "joy without --session and without JOY_SESSION has no AI session" {
    setup_human_auth
    setup_ai_session ai:test@joy
    unset JOY_SESSION

    run joy auth status
    [ "$status" -eq 0 ]
    [[ "$output" != *"ai:test@joy"* ]]
}
