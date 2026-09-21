#!/usr/bin/env bats
# JOY-02AE-84: prerelease-aware `joy release bump` / `joy release record`
# (keyword bump from a prerelease base, prerelease labels, explicit-version
# validation, SemVer 2.0 ordering, and item rollup into the final release).

load setup

@test "joy release bump with a prerelease label writes the suffixed version" {
    joy init --name "T" >/dev/null
    echo '{"name":"app","version":"0.0.0"}' > package.json
    joy project set release.version-files package.json >/dev/null

    run joy release bump minor alpha
    [ "$status" -eq 0 ]
    [[ "$output" == *"v0.0.0 -> v0.1.0-alpha"* ]]
    grep -q '"version":"0.1.0-alpha"' package.json
    [[ "$output" == *"joy release record minor alpha"* ]]
}

@test "joy release bump accepts the prerelease label via --prerelease" {
    joy init --name "T" >/dev/null
    echo '{"name":"app","version":"0.0.0"}' > package.json
    joy project set release.version-files package.json >/dev/null

    run joy release bump minor --prerelease beta
    [ "$status" -eq 0 ]
    [[ "$output" == *"v0.0.0 -> v0.1.0-beta"* ]]
    grep -q '"version":"0.1.0-beta"' package.json
}

@test "a second alpha bump increments the prerelease counter" {
    joy init --name "T" >/dev/null
    echo '{"name":"app","version":"0.1.0-alpha"}' > package.json
    joy project set release.version-files package.json >/dev/null

    run joy release bump minor alpha --adopt
    [ "$status" -eq 0 ]
    [[ "$output" == *"v0.1.0-alpha -> v0.1.0-alpha.1"* ]]
    grep -q '"version":"0.1.0-alpha.1"' package.json
}

@test "a keyword bump from a prerelease base graduates to stable instead of skipping a version" {
    joy init --name "T" >/dev/null
    echo '{"name":"app","version":"0.30.0-beta"}' > package.json
    joy project set release.version-files package.json >/dev/null

    run joy release bump patch --adopt
    [ "$status" -eq 0 ]
    [[ "$output" == *"v0.30.0-beta -> v0.30.0"* ]]
    grep -q '"version":"0.30.0"' package.json
}

@test "joy release bump refuses an explicit version that is not a version" {
    joy init --name "T" >/dev/null
    echo '{"name":"app","version":"0.0.0"}' > package.json
    joy project set release.version-files package.json >/dev/null

    run joy release bump 9lives
    [ "$status" -ne 0 ]
    [[ "$output" == *"invalid version: '9lives' is not a valid semver version"* ]]
    [[ "$output" == *"e.g. 1.2.3 or 1.2.3-beta"* ]]
}

@test "joy release ls sorts a stable release above its own prereleases" {
    joy init --name "T" >/dev/null
    git add -A && git commit -m "init [no-item]" --quiet

    printf 'y\n' | joy release record minor alpha >/dev/null
    printf 'y\n' | joy release record minor beta >/dev/null
    printf 'y\n' | joy release record minor >/dev/null

    run joy release ls
    [ "$status" -eq 0 ]
    # v0.1.0 (stable) must be listed before its own -alpha / -beta
    # prereleases: SemVer 2.0 precedence, not lexicographic order.
    stable_line=$(echo "$output" | grep -n "v0.1.0 " | cut -d: -f1)
    alpha_line=$(echo "$output" | grep -n "v0.1.0-alpha" | cut -d: -f1)
    beta_line=$(echo "$output" | grep -n "v0.1.0-beta" | cut -d: -f1)
    [ -n "$stable_line" ]
    [ -n "$alpha_line" ]
    [ -n "$beta_line" ]
    [ "$stable_line" -lt "$alpha_line" ]
    [ "$stable_line" -lt "$beta_line" ]
}

@test "an item closed during a prerelease phase rolls up into the final stable release" {
    joy init --name "T" >/dev/null

    run joy release bump minor alpha
    [ "$status" -eq 0 ]
    printf 'y\n' | joy release record minor alpha >/dev/null

    id=$(joy add task "ship it" | grep -oiE '[A-Z]+-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{2}' | head -1)
    joy start "$id" >/dev/null
    joy submit "$id" >/dev/null
    joy approve "$id" >/dev/null 2>&1 || true
    joy close "$id" >/dev/null 2>&1 || true

    run joy release bump minor beta
    [ "$status" -eq 0 ]
    printf 'y\n' | joy release record minor beta >/dev/null

    run joy release bump minor
    [ "$status" -eq 0 ]
    run bash -c "printf 'y\n' | joy release record minor"
    [ "$status" -eq 0 ]

    run joy release show v0.1.0
    [ "$status" -eq 0 ]
    [[ "$output" == *"$id"* ]]
}
