#!/usr/bin/env bats
# JOY-0164-B5: joy update + per-clone version marker + auto-sync.

load setup

@test "joy update --check guides a package-manager install to the right upgrade command" {
    # The test binary has no axoupdater receipt, so it reads as foreign-managed.
    joy init --name "Test"
    git add -A && git commit -m "init [no-item]" --quiet
    run joy update --check
    [[ "$output" == *"managed by"* ]]
    [[ "$output" == *"upgrade with:"* ]]
    [[ "$output" == *"curl -fsSL get.joyint.com/joy | sh"* ]]
    [[ "$output" == *"re-synced automatically"* ]]
}

@test "joy update points a package-manager install at the upgrade command, no binary swap" {
    joy init --name "Test"
    git add -A && git commit -m "init [no-item]" --quiet
    run joy update
    [ "$status" -eq 0 ]
    [[ "$output" == *"managed by"* ]]
    [[ "$output" == *"upgrade with: curl -fsSL get.joyint.com/joy | sh"* ]]
    [[ "$output" == *"re-synced automatically after you upgrade"* ]]
}

@test "joy update --check reports the version marker in sync after a fresh init" {
    joy init --name "Test"
    git add -A && git commit -m "init [no-item]" --quiet
    run joy update --check
    [[ "$output" == *"version marker"* ]]
    [[ "$output" == *"All Joy-managed state is up to date"* ]]
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

@test "auto-sync keeps --json stdout clean (JOY-01FF-36)" {
    joy init --name "Test"
    git add -A && git commit -m "init [no-item]" --quiet
    joy ls >/dev/null  # initial stamp
    # Force a stale marker so the auto-sync fires on the next invocation.
    git config --local joy.last-sync-version "0.0.0-stale"
    # Capture stdout only: the auto-sync summary must not corrupt the payload.
    out=$(joy --json ls 2>/dev/null)
    [ "${out:0:1}" = "{" ]
    [ "${out: -1}" = "}" ]
    [[ "$out" != *"version marker"* ]] || bats_lib_diag "sync summary leaked to --json stdout: $out"
    # The sync still ran (marker refreshed), just without the stdout table.
    [ "$(git config --local joy.last-sync-version)" != "0.0.0-stale" ]
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

# What a sync changes is committed as one commit.

# A project whose files are committed and whose clone an older joy synced
# last, with SECURITY.md as that joy left it and a file of the person's.
stale_project() {
    joy init --name "Test"
    joy ls >/dev/null 2>&1
    git add -A && git commit -m "init [no-item]" --quiet
    echo "what an older joy wrote" > SECURITY.md
    git add -A && git commit -m "older [no-item]" --quiet
    git config --local joy.last-sync-version "0.0.1"
    echo "mine" > notes.txt
}

@test "joy update --force commits what it changed as one commit" {
    stale_project
    commits=$(git rev-list --count HEAD)
    run joy update --no-binary --force
    [ "$status" -eq 0 ]
    [[ "$output" == *"Committed as "* ]]
    [ "$(git rev-list --count HEAD)" -eq $((commits + 1)) ]
    [[ "$(git log -1 --format=%s)" == "chore(joy): project files updated from joy 0.0.1 to joy "*" [no-item]" ]]
    [[ "$(git show --stat --format= HEAD)" == *"SECURITY.md"* ]]
    # the person's file is not in the commit and still theirs
    [[ "$(git show --stat --format= HEAD)" != *"notes.txt"* ]]
    [ "$(git status --porcelain)" = "?? notes.txt" ]
}

@test "joy update without a terminal commits without asking" {
    stale_project
    commits=$(git rev-list --count HEAD)
    run joy update --no-binary </dev/null
    [ "$status" -eq 0 ]
    [[ "$output" != *"[Y/n]"* ]]
    [[ "$output" == *"Committed as "* ]]
    [ "$(git rev-list --count HEAD)" -eq $((commits + 1)) ]
}

@test "joy update --json names the commit" {
    stale_project
    out=$(joy --json update --no-binary 2>/dev/null)
    [[ "$out" == *"\"committed\""* ]]
    [[ "$out" == *"$(git rev-parse HEAD)"* ]]
}

@test "joy update leaves a file alone that the person had already changed" {
    stale_project
    echo "what an older joy wrote, edited by hand" > SECURITY.md
    head=$(git rev-parse HEAD)
    run joy update --no-binary --force
    [ "$status" -eq 0 ]
    [[ "$output" == *"Left for you to commit, changed before the sync: SECURITY.md"* ]]
    # whatever else the update committed, this file is not in it
    [[ "$(git diff --name-only "$head" HEAD)" != *"SECURITY.md"* ]]
    [[ "$(git status --porcelain)" == *"SECURITY.md"* ]]
}

@test "joy update commits nothing with workflow.auto-git off" {
    stale_project
    # written by hand: a joy call here would run the sync before the switch
    printf 'version: 1\nworkflow:\n  auto-git: "off"\n' > .joy/config.yaml
    commits=$(git rev-list --count HEAD)
    run joy update --no-binary --force
    [ "$status" -eq 0 ]
    [[ "$output" == *"Not committed: workflow.auto-git is off."* ]]
    [ "$(git rev-list --count HEAD)" -eq "$commits" ]
}

@test "the sync after a version change commits what it changed and names the commit" {
    stale_project
    commits=$(git rev-list --count HEAD)
    run --separate-stderr joy ls
    [ "$status" -eq 0 ]
    [[ "$stderr" == *"synced this repo (previous marker: 0.0.1), committed as "* ]]
    [ "$(git rev-list --count HEAD)" -eq $((commits + 1)) ]
    [[ "$(git log -1 --format=%s)" == "chore(joy): project files updated from joy 0.0.1 to joy "* ]]
    [ "$(git status --porcelain)" = "?? notes.txt" ]
    # and the next call finds nothing to do
    run --separate-stderr joy ls
    [[ "$stderr" != *"synced this repo"* ]]
    [ "$(git rev-list --count HEAD)" -eq $((commits + 1)) ]
}
