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
    [[ "$output" == \"joy_t_*\" ]]
}

@test "--passphrase-stdin strips the trailing newline correctly" {
    joy init --name "T" >/dev/null
    # `printf` without a newline still works because EOF on stdin
    # closes the line read.
    run bash -c "printf '%s' '$TEST_PASSPHRASE' | joy auth init --passphrase-stdin"
    [ "$status" -eq 0 ]
}

@test "joy project member add accepts --passphrase-stdin" {
    # Sessions are TTY-bound on purpose (identity::check_session): a
    # session minted in one terminal must not be usable from another
    # terminal or from an unattended subprocess. So this test cannot
    # mix `setup_human_auth` (TTY stdin) with `echo X | joy …` (pipe
    # stdin) -- the pipe call would correctly be refused.
    #
    # A realistic `--passphrase-stdin` scenario is a GUI / CI /
    # orchestration pipeline where *every* joy invocation reads from
    # a pipe. Wrap auth init and the member add in the same pipe
    # context so the two `current_tty()` values match (both None).
    run bash -c "
        joy init --name 'Test Project' >/dev/null
        echo '$TEST_PASSPHRASE' | joy auth init --passphrase-stdin >/dev/null
        echo '$TEST_PASSPHRASE' | joy project member add ai:pstdin@joy --passphrase-stdin
    "
    [ "$status" -eq 0 ]
    [[ "$output" == *"Added member ai:pstdin@joy"* ]]
}
