#!/usr/bin/env bats
# JOY-01CD-D5: joy ai reset must not remove an in-use AI member from
# project.yaml, and must never mutate project.yaml without confirmation.
# A member is only removed when it is orphaned (no operator delegates it),
# and the removal is gated by [y/N] / --force. Non-interactive runs refuse
# instead of silently changing shared, versioned state.

load setup

@test "joy ai reset refuses to change project.yaml without confirmation" {
    setup_human_auth
    joy project member add ai:claude@joy --passphrase "$TEST_PASSPHRASE"

    # No local .claude config exists; the member is present. A non-interactive
    # run without --force must refuse, and must leave project.yaml untouched.
    run joy ai reset --tool claude </dev/null
    [ "$status" -ne 0 ]
    grep -q "ai:claude@joy" .joy/project.yaml
}

@test "joy ai reset --force removes an orphaned member and leaves no dangling delegation" {
    setup_human_auth
    joy project member add ai:claude@joy --passphrase "$TEST_PASSPHRASE"
    # Issue a delegation so the caller (test@example.com) is the sole delegator.
    joy auth token add ai:claude@joy --passphrase "$TEST_PASSPHRASE" >/dev/null
    grep -q "ai_delegations" .joy/project.yaml

    run joy ai reset --tool claude --force </dev/null
    [ "$status" -eq 0 ]
    # Member gone, and the caller's now-empty delegation map is not left behind.
    ! grep -q "ai:claude@joy" .joy/project.yaml
    ! grep -q "ai_delegations" .joy/project.yaml
}

@test "joy ai reset --tool leaves an unrelated tool's member intact" {
    setup_human_auth
    joy project member add ai:claude@joy --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:qwen@joy --passphrase "$TEST_PASSPHRASE"

    run joy ai reset --tool claude --force </dev/null
    [ "$status" -eq 0 ]
    ! grep -q "ai:claude@joy" .joy/project.yaml
    grep -q "ai:qwen@joy" .joy/project.yaml
}
