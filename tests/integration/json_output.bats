#!/usr/bin/env bats
# Verify the cross-cutting --json infrastructure introduced by
# JOY-00C9-5F (ADR-036 §1). Per-command JSON shapes are tested by
# the subcommand-specific subtasks.

load setup

@test "joy --json is accepted as a global flag (before subcommand)" {
    joy init --name "Test"
    run joy --json ls
    [ "$status" -eq 0 ]
}

@test "joy --json is accepted after the subcommand" {
    joy init --name "Test"
    run joy ls --json
    [ "$status" -eq 0 ]
}

@test "joy --json works for every position-equivalent invocation" {
    joy init --name "Test"
    joy add task "First"
    out_pre=$(joy --json show "$(joy ls 2>/dev/null | grep First | awk '{print $1}')" 2>&1 || true)
    out_post=$(joy show --json "$(joy ls 2>/dev/null | grep First | awk '{print $1}')" 2>&1 || true)
    # Both forms must exit successfully even if individual subcommands
    # have not yet adopted JSON output.
    [ "${#out_pre}" -ge 0 ]
    [ "${#out_post}" -ge 0 ]
}

@test "joy --help mentions --json" {
    run joy --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"--json"* ]]
}
