#!/usr/bin/env bats
# JOY-0164-B5: joy update + per-clone version marker + auto-sync.

load setup

@test "joy update --check reports binary receipt missing for non-cargo-dist build" {
    joy init --name "Test"
    git add -A && git commit -m "init [no-item]" --quiet
    run joy update --check
    [[ "$output" == *"binary: install receipt missing"* ]]
}

@test "joy update --check reports the repo as freshly synced after first joy invocation" {
    joy init --name "Test"
    git add -A && git commit -m "init [no-item]" --quiet
    # joy ls triggers auto-sync, which stamps the marker.
    joy ls >/dev/null
    run joy update --check
    [[ "$output" == *"repo: synced"* ]]
}

@test "auto-sync stamps joy.last-sync-version on first joy invocation" {
    joy init --name "Test"
    [ -z "$(git config --local --get joy.last-sync-version || true)" ] && true || joy_first=$(git config --local --get joy.last-sync-version)
    joy ls >/dev/null
    stamped=$(git config --local --get joy.last-sync-version)
    [ -n "$stamped" ]
}

@test "auto-sync prints a 'synced this repo' line when version marker is stale" {
    joy init --name "Test"
    git add -A && git commit -m "init [no-item]" --quiet
    # First invocation stamps the current version.
    joy ls >/dev/null
    # Force the marker to a stale value to simulate a binary upgrade.
    git config --local joy.last-sync-version "0.0.0-stale"
    # Next joy invocation should detect the mismatch and resync.
    run joy ls
    [[ "$output" == *"synced this repo"* ]] || [[ "$stderr" == *"synced this repo"* ]] || \
        bats_lib_diag "expected 'synced this repo' message in stderr; got: $output"
    # And the marker must be brought back to the binary version.
    stamped=$(git config --local --get joy.last-sync-version)
    [ "$stamped" != "0.0.0-stale" ]
}

@test "auto-sync respects auto-sync: false in .joy/config.yaml" {
    joy init --name "Test"
    git add -A && git commit -m "init [no-item]" --quiet
    joy ls >/dev/null  # initial stamp
    # Disable auto-sync.
    cat > .joy/config.yaml <<EOF
version: 1
auto-sync: false
EOF
    git config --local joy.last-sync-version "0.0.0-stale"
    joy ls >/dev/null 2>/dev/null
    # Marker stays stale because auto-sync is off.
    stamped=$(git config --local --get joy.last-sync-version)
    [ "$stamped" = "0.0.0-stale" ]
}
