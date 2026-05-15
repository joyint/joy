#!/usr/bin/env bats
# joy auth --passphrase-stdin: read the passphrase from a single stdin
# line (JOY-018E-21). Pattern matches `gh auth login --with-token` and
# `docker login --password-stdin`.

load setup

@test "joy auth init accepts the passphrase via --passphrase-stdin" {
    joy init --name "T" >/dev/null
    run bash -c "echo '$TEST_PASSPHRASE' | joy auth init --passphrase-stdin"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Authentication initialized"* ]]
}

@test "joy auth (re-login) accepts --passphrase-stdin after init" {
    setup_human_auth
    joy deauth >/dev/null
    run bash -c "echo '$TEST_PASSPHRASE' | joy auth --passphrase-stdin"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Authenticated as"* ]]
}

@test "--passphrase and --passphrase-stdin are mutually exclusive" {
    setup_human_auth
    joy deauth >/dev/null
    run bash -c "echo '$TEST_PASSPHRASE' | joy auth --passphrase '$TEST_PASSPHRASE' --passphrase-stdin"
    [ "$status" -ne 0 ]
    [[ "$output" == *"mutually exclusive"* ]]
}

@test "--passphrase-stdin rejects empty input" {
    setup_human_auth
    joy deauth >/dev/null
    run bash -c "joy auth --passphrase-stdin < /dev/null"
    [ "$status" -ne 0 ]
    [[ "$output" == *"--passphrase-stdin"* ]]
}

@test "--passphrase-stdin works for joy auth token add" {
    setup_human_auth
    joy project member add ai:stdin@joy --passphrase "$TEST_PASSPHRASE" >/dev/null
    run bash -c "echo '$TEST_PASSPHRASE' | joy auth token add ai:stdin@joy --passphrase-stdin"
    [ "$status" -eq 0 ]
    [[ "$output" == joy_t_* ]]
}

@test "--passphrase-stdin strips the trailing newline correctly" {
    joy init --name "T" >/dev/null
    # `printf` without a newline still works because EOF on stdin
    # closes the line read.
    run bash -c "printf '%s' '$TEST_PASSPHRASE' | joy auth init --passphrase-stdin"
    [ "$status" -eq 0 ]
}

@test "joy project member add accepts --passphrase-stdin" {
    setup_human_auth
    run bash -c "echo '$TEST_PASSPHRASE' | joy project member add ai:pstdin@joy --passphrase-stdin"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Added member ai:pstdin@joy"* ]]
}
