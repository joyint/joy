#!/usr/bin/env bats
# JOY-00F9-D7: CLI surface for release.version-files in joy project set/get.

load setup

@test "joy project set release.version-files <CSV> replaces the whole list" {
    joy init --name "T" >/dev/null
    run joy project set release.version-files "crates/a/Cargo.toml,crates/b/Cargo.toml"
    [ "$status" -eq 0 ]

    run joy project get release.version-files
    [ "$status" -eq 0 ]
    [[ "${lines[0]}" == "crates/a/Cargo.toml" ]]
    [[ "${lines[1]}" == "crates/b/Cargo.toml" ]]
}

@test "joy project set release.version-files with empty string clears the list" {
    joy init --name "T" >/dev/null
    joy project set release.version-files "crates/a/Cargo.toml" >/dev/null
    run joy project set release.version-files ""
    [ "$status" -eq 0 ]

    run joy project get release.version-files
    [ "$status" -ne 0 ]

    # `release:` block is removed from the YAML when it becomes empty.
    run grep -q "^release:" .joy/project.yaml
    [ "$status" -ne 0 ]
}

@test "joy project set release.version-files --add appends a single entry" {
    joy init --name "T" >/dev/null
    joy project set release.version-files "crates/a/Cargo.toml" >/dev/null
    run joy project set release.version-files --add crates/b/Cargo.toml
    [ "$status" -eq 0 ]
    [[ "$output" == *"release.version-files += crates/b/Cargo.toml"* ]]

    run joy project get release.version-files
    [ "${lines[0]}" = "crates/a/Cargo.toml" ]
    [ "${lines[1]}" = "crates/b/Cargo.toml" ]
}

@test "joy project set release.version-files --add is idempotent and warns on duplicates" {
    joy init --name "T" >/dev/null
    joy project set release.version-files --add crates/a/Cargo.toml >/dev/null
    run joy project set release.version-files --add crates/a/Cargo.toml
    [ "$status" -eq 0 ]
    [[ "$output" == *"warning"* ]]
    [[ "$output" == *"already configured"* ]]
}

@test "joy project set release.version-files --rm removes a configured entry" {
    joy init --name "T" >/dev/null
    joy project set release.version-files "crates/a/Cargo.toml,crates/b/Cargo.toml" >/dev/null
    run joy project set release.version-files --rm crates/a/Cargo.toml
    [ "$status" -eq 0 ]
    [[ "$output" == *"release.version-files -= crates/a/Cargo.toml"* ]]

    run joy project get release.version-files
    [ "${#lines[@]}" -eq 1 ]
    [ "${lines[0]}" = "crates/b/Cargo.toml" ]
}

@test "joy project set release.version-files --rm fails on unknown entry" {
    joy init --name "T" >/dev/null
    joy project set release.version-files "crates/a/Cargo.toml" >/dev/null
    run joy project set release.version-files --rm crates/nope/Cargo.toml
    [ "$status" -ne 0 ]
    [[ "$output" == *"is not configured"* ]]
}

@test "joy project get release.version-files --json returns an array of strings" {
    joy init --name "T" >/dev/null
    joy project set release.version-files "crates/a/Cargo.toml,crates/b/Cargo.toml" >/dev/null
    run joy project get release.version-files --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.key == "release.version-files"' >/dev/null
    echo "$output" | jq -e '.data.value | type == "array"' >/dev/null
    echo "$output" | jq -e '.data.value | length == 2' >/dev/null
    echo "$output" | jq -e '.data.value[0] == "crates/a/Cargo.toml"' >/dev/null
}

@test "joy project get release.version-files --json returns null when unset" {
    joy init --name "T" >/dev/null
    run joy project get release.version-files --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.value == null' >/dev/null
}

@test "round-trip preserves mapping-form entries" {
    joy init --name "T" >/dev/null
    cat >> .joy/project.yaml <<'EOF'
release:
  version-files:
    - crates/plain.toml
    - path: crates/mapped.toml
      extra: keep-me
EOF
    joy project set release.version-files --add crates/new.toml >/dev/null

    # mapping-form entry still has its `extra` field after the add
    grep -A1 "path: crates/mapped.toml" .joy/project.yaml | grep -q "extra: keep-me"

    # rm by path matches mapping-form entries
    run joy project set release.version-files --rm crates/mapped.toml
    [ "$status" -eq 0 ]
    run grep -q "path: crates/mapped.toml" .joy/project.yaml
    [ "$status" -ne 0 ]
}

@test "--add is rejected on a scalar key" {
    joy init --name "T" >/dev/null
    run joy project set forge --add github
    [ "$status" -ne 0 ]
    [[ "$output" == *"not a list-typed key"* ]] || [[ "$output" == *"conflicts_with"* ]] || [[ "$output" == *"cannot be used"* ]]
}

@test "--add and --rm cannot be combined with a positional value" {
    joy init --name "T" >/dev/null
    run joy project set release.version-files crates/a/Cargo.toml --add crates/b/Cargo.toml
    [ "$status" -ne 0 ]
}

@test "joy project get release.version-files --describe annotates the first line" {
    joy init --name "T" >/dev/null
    joy project set release.version-files "crates/a/Cargo.toml" >/dev/null
    run joy project get release.version-files --describe
    [ "$status" -eq 0 ]
    [[ "$output" == *"joy release bump"* ]]
}

@test "joy project get '*' --describe includes release.version-files" {
    joy init --name "T" >/dev/null
    joy project set release.version-files "crates/a/Cargo.toml" >/dev/null
    run joy project get '*' --describe
    [ "$status" -eq 0 ]
    [[ "$output" == *"release.version-files"* ]]
}
