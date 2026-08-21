#!/usr/bin/env bats
# JOY-019B-C8: joy project set <KEY> opens $EDITOR when called without VALUE.

load setup

# Write a fake-editor script that replaces the file with the given content.
fake_editor_writing() {
    local script="$TEST_DIR/fake-editor.sh"
    cat > "$script" <<EOF
#!/usr/bin/env bash
cat > "\$1" <<'BODY'
$1
BODY
EOF
    chmod +x "$script"
    echo "$script"
}

# Write a fake-editor script that leaves the file untouched.
fake_editor_noop() {
    local script="$TEST_DIR/fake-editor-noop.sh"
    cat > "$script" <<'EOF'
#!/usr/bin/env bash
# leave the file unchanged
true
EOF
    chmod +x "$script"
    echo "$script"
}

# Write a fake-editor script that empties the file.
fake_editor_empty() {
    local script="$TEST_DIR/fake-editor-empty.sh"
    cat > "$script" <<'EOF'
#!/usr/bin/env bash
truncate -s 0 "$1"
EOF
    chmod +x "$script"
    echo "$script"
}

@test "joy project set <scalar> with no VALUE opens editor; saved content applies" {
    joy init --name "T" >/dev/null
    local editor
    editor=$(fake_editor_writing "new-description")
    run joy project set description --editor "$editor"
    [ "$status" -eq 0 ]

    run joy project get description
    [ "$status" -eq 0 ]
    [[ "$output" == "new-description" ]]
}

@test "joy project set <scalar> with no VALUE: empty buffer clears the field" {
    joy init --name "T" >/dev/null
    joy project set description "to-be-cleared" >/dev/null
    local editor
    editor=$(fake_editor_empty)
    run joy project set description --editor "$editor"
    [ "$status" -eq 0 ]

    run joy project get description
    [ "$status" -ne 0 ]
    # The `description:` line is pruned from the YAML on clear.
    run grep -q "^description:" .joy/project.yaml
    [ "$status" -ne 0 ]
}

@test "joy project set <scalar> with no VALUE: unchanged buffer is a no-op" {
    joy init --name "T" >/dev/null
    joy project set description "stable" >/dev/null
    local editor
    editor=$(fake_editor_noop)
    run joy project set description --editor "$editor"
    [ "$status" -eq 0 ]
    [[ "$output" == *"unchanged"* ]]

    run joy project get description
    [[ "$output" == "stable" ]]
}

@test "joy project set forge via editor still runs supported-value validation" {
    joy init --name "T" >/dev/null
    local editor
    editor=$(fake_editor_writing "sourcehut")
    run joy project set forge --editor "$editor"
    [ "$status" -ne 0 ]
    [[ "$output" == *"unsupported forge 'sourcehut'"* ]]
}

@test "joy project set release.version-files with no VALUE opens editor (list)" {
    joy init --name "T" >/dev/null
    cat > "$TEST_DIR/fake-editor.sh" <<'EOF'
#!/usr/bin/env bash
cat > "$1" <<'BODY'
# header
crates/a/Cargo.toml

crates/b/Cargo.toml
BODY
EOF
    chmod +x "$TEST_DIR/fake-editor.sh"

    run joy project set release.version-files --editor "$TEST_DIR/fake-editor.sh"
    [ "$status" -eq 0 ]

    run joy project get release.version-files
    [ "$status" -eq 0 ]
    [ "${lines[0]}" = "crates/a/Cargo.toml" ]
    [ "${lines[1]}" = "crates/b/Cargo.toml" ]
}

@test "joy project set release.version-files via editor: empty buffer clears the list" {
    joy init --name "T" >/dev/null
    joy project set release.version-files "crates/a/Cargo.toml" >/dev/null

    cat > "$TEST_DIR/fake-editor.sh" <<'EOF'
#!/usr/bin/env bash
truncate -s 0 "$1"
EOF
    chmod +x "$TEST_DIR/fake-editor.sh"

    run joy project set release.version-files --editor "$TEST_DIR/fake-editor.sh"
    [ "$status" -eq 0 ]

    run joy project get release.version-files
    [ "$status" -ne 0 ]
}

@test "joy project set release.version-files via editor: unchanged buffer is a no-op" {
    joy init --name "T" >/dev/null
    joy project set release.version-files "crates/a/Cargo.toml" >/dev/null

    cat > "$TEST_DIR/fake-editor.sh" <<'EOF'
#!/usr/bin/env bash
true
EOF
    chmod +x "$TEST_DIR/fake-editor.sh"

    run joy project set release.version-files --editor "$TEST_DIR/fake-editor.sh"
    [ "$status" -eq 0 ]
    [[ "$output" == *"unchanged"* ]]
}
