#!/usr/bin/env bats
# prepare-commit-msg hook: pre-fills the commit message with the matching
# Joy item id, without ever leaving a placeholder to delete (JOY-01B1-FF).

load setup

# The hook shims out to `joy prepare-commit-msg`; make sure the binary under
# test is the one on PATH (setup.bash already prepends target/debug).

@test "prepares a complete subject from a staged item file" {
    setup_human_auth
    joy add task "wire up the frobnicator" >/dev/null
    ID=$(joy ls 2>/dev/null | grep frobnicator | awk '{print $1}')
    joy start "$ID" >/dev/null
    git add -A

    msgfile="$TEST_DIR/MSG"
    printf '\n# git help line\n' > "$msgfile"
    run joy prepare-commit-msg "$msgfile" ""
    [ "$status" -eq 0 ]

    # First line is a complete conventional subject ending in the id.
    head -1 "$msgfile" | grep -qE "^(feat|fix|rework|docs).*\[$ID\]$"
    # No placeholder text the user would have to delete.
    ! grep -q "<type>" "$msgfile"
    ! grep -q "<describe" "$msgfile"
    # git's own comment block is preserved.
    grep -q "# git help line" "$msgfile"
}

@test "single in-progress item used when nothing staged" {
    setup_human_auth
    joy add task "lonely task" >/dev/null
    ID=$(joy ls 2>/dev/null | grep "lonely task" | awk '{print $1}')
    joy start "$ID" >/dev/null
    # commit the item file so it is no longer staged; item stays in-progress
    git add -A
    git commit -q --no-verify -m "setup [no-item]"
    # a fresh code-only change, nothing from .joy/ staged
    mkdir -p crates/joy-cli/src
    echo "// x" >> crates/joy-cli/src/x.rs
    git add crates/joy-cli/src/x.rs

    msgfile="$TEST_DIR/MSG"
    printf '\n# c\n' > "$msgfile"
    run joy prepare-commit-msg "$msgfile" ""
    [ "$status" -eq 0 ]
    head -1 "$msgfile" | grep -qE "\[$ID\]$"
}

@test "ambiguous: empty subject and commented candidates" {
    setup_human_auth
    joy add task "task aaa" >/dev/null
    joy add task "task bbb" >/dev/null
    A=$(joy ls 2>/dev/null | grep "task aaa" | awk '{print $1}')
    B=$(joy ls 2>/dev/null | grep "task bbb" | awk '{print $1}')
    joy start "$A" >/dev/null
    joy start "$B" >/dev/null
    git add -A
    git commit -q --no-verify -m "setup [no-item]"
    echo "// y" >> file.txt
    git add file.txt

    msgfile="$TEST_DIR/MSG"
    printf '\n# c\n' > "$msgfile"
    run joy prepare-commit-msg "$msgfile" ""
    [ "$status" -eq 0 ]
    # subject line empty
    [ -z "$(head -1 "$msgfile")" ]
    # both candidates present as commented lines
    grep -q "^# .*\[$A\]" "$msgfile"
    grep -q "^# .*\[$B\]" "$msgfile"
}

@test "does nothing when a source is given (e.g. -m)" {
    setup_human_auth
    joy add task "srctask" >/dev/null
    ID=$(joy ls 2>/dev/null | grep srctask | awk '{print $1}')
    joy start "$ID" >/dev/null
    git add -A

    msgfile="$TEST_DIR/MSG"
    printf 'my own message\n' > "$msgfile"
    run joy prepare-commit-msg "$msgfile" "message"
    [ "$status" -eq 0 ]
    # untouched
    [ "$(cat "$msgfile")" = "my own message" ]
}

@test "does not clobber a message the user already wrote" {
    setup_human_auth
    joy add task "keep mine" >/dev/null
    ID=$(joy ls 2>/dev/null | grep "keep mine" | awk '{print $1}')
    joy start "$ID" >/dev/null
    git add -A

    msgfile="$TEST_DIR/MSG"
    printf 'already written subject\n\n# git comment\n' > "$msgfile"
    run joy prepare-commit-msg "$msgfile" ""
    [ "$status" -eq 0 ]
    head -1 "$msgfile" | grep -q "already written subject"
}
