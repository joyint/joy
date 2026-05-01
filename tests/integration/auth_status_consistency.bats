#!/usr/bin/env bats
# joy project member auth check mark must agree with runtime auth.
# JOY-00F4-CF: previously the column showed a check mark even when
# the session would not be accepted in the current shell.

load setup

@test "session created via PTY does not show as authenticated outside that PTY" {
    setup_human_auth                    # session created in this shell's TTY
    rm -rf "$XDG_STATE_HOME/joy/sessions"

    # Create a session inside a separate PTY with a TTY device.
    pty_run "joy auth --passphrase '$TEST_PASSPHRASE'" >/dev/null 2>&1

    # Outside that PTY: joy auth status correctly reports no active session.
    run joy auth status
    [ "$status" -ne 0 ]

    # joy project member must agree: no auth check mark for this user.
    run joy project member
    [ "$status" -eq 0 ]
    # The column header is "Auth"; member row must not carry a check.
    line=$(echo "$output" | grep "test@example.com" || true)
    [[ "$line" != *"✓"* ]]
}

@test "AI member without JOY_SESSION env var does not show as authenticated" {
    setup_human_auth
    setup_ai_session ai:test@joy
    # Drop the env var; the session file remains on disk.
    unset JOY_SESSION

    run joy project member
    [ "$status" -eq 0 ]
    line=$(echo "$output" | grep "ai:test@joy" || true)
    [[ "$line" != *"✓ 🔐"* ]]
}
