#!/usr/bin/env bats
# JOY-019C-E2: joy release bump --adopt + vertical version-mismatch error.

load setup

@test "joy release bump --adopt picks up the version from the file" {
    joy init --name "T" >/dev/null
    echo '{"name":"app","version":"0.1.0"}' > package.json
    joy project set release.version-files package.json >/dev/null

    run joy release bump patch --adopt
    [ "$status" -eq 0 ]
    [[ "$output" == *"v0.1.0 -> v0.1.1"* ]]
    grep -q '"version":"0.1.1"' package.json
}

@test "joy release bump --adopt errors when configured files disagree" {
    joy init --name "T" >/dev/null
    echo '{"name":"a","version":"0.1.0"}' > a.json
    echo '{"name":"b","version":"0.2.0"}' > b.json
    joy project set release.version-files "a.json,b.json" >/dev/null

    run joy release bump patch --adopt
    [ "$status" -ne 0 ]
    [[ "$output" == *"disagree"* ]]
    [[ "$output" == *"a.json"* ]]
    [[ "$output" == *"b.json"* ]]
}

@test "joy release bump --adopt errors when no version can be detected" {
    joy init --name "T" >/dev/null
    echo 'plain text with no version' > note.txt
    joy project set release.version-files note.txt >/dev/null

    run joy release bump patch --adopt
    [ "$status" -ne 0 ]
    [[ "$output" == *"could not detect a version"* ]]
    [[ "$output" == *"note.txt"* ]]
}

@test "joy release bump without --adopt: mismatch error includes detected version" {
    joy init --name "T" >/dev/null
    echo '{"name":"app","version":"0.1.0"}' > package.json
    joy project set release.version-files package.json >/dev/null

    run joy release bump patch
    [ "$status" -ne 0 ]
    [[ "$output" == *"! package.json"* ]]
    [[ "$output" == *"expected: 0.0.0"* ]]
    [[ "$output" == *"found:    0.1.0"* ]]
    [[ "$output" == *"version mismatch (1 of 1 file)"* ]]
    [[ "$output" == *"joy release bump --adopt"* ]]
    [[ "$output" == *"joy release record 0.1.0"* ]]
}

@test "joy release bump mismatch error stays clean when detection fails" {
    joy init --name "T" >/dev/null
    echo 'no version anywhere' > weird.txt
    joy project set release.version-files weird.txt >/dev/null

    run joy release bump patch
    [ "$status" -ne 0 ]
    [[ "$output" == *"! weird.txt"* ]]
    [[ "$output" == *"found:    (no version detected)"* ]]
    # The record hint must not invent a number when nothing was found.
    [[ "$output" == *"joy release record <X.Y.Z>"* ]]
    [[ "$output" != *"joy release record 0."* ]]
}

@test "joy release bump every line in the mismatch block stays under 60 chars" {
    joy init --name "T" >/dev/null
    echo '{"name":"a","version":"0.1.0"}' > pkg.json
    joy project set release.version-files pkg.json >/dev/null

    run joy release bump patch
    [ "$status" -ne 0 ]

    # Strip ANSI color codes before measuring width.
    while IFS= read -r line; do
        local stripped
        stripped=$(printf '%s' "$line" | sed 's/\x1b\[[0-9;]*m//g')
        local len=${#stripped}
        [ "$len" -le 60 ] || {
            echo "line >60 chars: '$stripped' (len $len)"
            false
        }
    done <<< "$output"
}
