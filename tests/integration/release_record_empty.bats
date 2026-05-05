#!/usr/bin/env bats
# JOY-0163-95: `joy release record` must produce a .yaml even when
# no items have been closed since the previous release, so that
# `joy release publish` is idempotent across submodules with nothing
# to ship.

load setup

@test "joy release record creates an empty .yaml when nothing has been closed" {
    joy init --name "Test"
    git add -A && git commit -m "init [no-item]" --quiet

    run joy release record patch
    [ "$status" -eq 0 ]
    [[ "$output" == *"recording empty release v0.0.1"* ]]

    [ -f ".joy/releases/T-v0.0.1.yaml" ] || [ -f ".joy/releases/JOY-v0.0.1.yaml" ] || ls .joy/releases/*-v0.0.1.yaml >/dev/null

    # Tag was created locally, no prompt was reached.
    git tag -l | grep -q "^v0.0.1$"
}

@test "joy release record does not prompt for an empty release" {
    joy init --name "Test"
    git add -A && git commit -m "init [no-item]" --quiet

    # Run with </dev/null so that any unexpected prompt would fail with
    # EOF on read instead of hanging the test.
    run bash -c "joy release record patch </dev/null"
    [ "$status" -eq 0 ]
    [[ "$output" != *"Aborted"* ]]
}
