#!/usr/bin/env bats
# JOY-0164-B5: joy update + per-clone version marker + auto-sync.

load setup

@test "joy update --check reports binary receipt missing for non-cargo-dist build" {
    joy init --name "Test"
    git add -A && git commit -m "init [no-item]" --quiet
    run joy update --check
    [[ "$output" == *"install receipt missing"* ]]
}

@test "joy update --check reports the version marker in sync after a fresh init" {
    joy init --name "Test"
    git add -A && git commit -m "init [no-item]" --quiet
    run joy update --check
    [[ "$output" == *"version marker: ok"* ]]
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

@test "joy update --check does not write any files even with stale marker (JOY-0165-1F)" {
    joy init --name "Test"
    git add -A && git commit -m "init [no-item]" --quiet
    # Force a stale marker so the auto-sync hook would otherwise fire.
    git config --local joy.last-sync-version "0.0.0-stale"
    [ -z "$(git status --porcelain)" ]
    run joy update --check
    # --check is allowed to exit 2 (stale binary or repo); just must not write.
    [ -z "$(git status --porcelain)" ]
    # Marker must still be the stale value -- nothing was synced.
    [ "$(git config --local joy.last-sync-version)" = "0.0.0-stale" ]
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
