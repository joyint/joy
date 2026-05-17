#!/usr/bin/env bats
# JOY-019A-1E: read commands piped into a short-circuiting consumer
# (head, less ... q) must not panic on stderr when the pipe closes.

load setup

assert_no_panic() {
    local stderr_file="$1"
    run cat "$stderr_file"
    [[ "$output" != *"panicked at"* ]] || {
        echo "stderr contained a panic:"
        cat "$stderr_file"
        false
    }
    [[ "$output" != *"Broken pipe"* ]] || {
        echo "stderr mentioned Broken pipe:"
        cat "$stderr_file"
        false
    }
}

@test "joy show piped to head leaves stderr clean" {
    setup_human_auth
    joy add task "Some task" >/dev/null
    local id
    id=$(joy ls --json | jq -r '.data.items[0].id')
    local stderr_file="$TEST_DIR/stderr"
    joy show "$id" 2>"$stderr_file" | head -1 >/dev/null
    assert_no_panic "$stderr_file"
}

@test "joy ls piped to head leaves stderr clean" {
    setup_human_auth
    for i in 1 2 3 4 5; do
        joy add task "Task $i" >/dev/null
    done
    local stderr_file="$TEST_DIR/stderr"
    joy ls 2>"$stderr_file" | head -2 >/dev/null
    assert_no_panic "$stderr_file"
}

@test "joy log piped to head leaves stderr clean" {
    setup_human_auth
    for i in 1 2 3 4 5; do
        joy add task "Task $i" >/dev/null
    done
    local stderr_file="$TEST_DIR/stderr"
    joy log 2>"$stderr_file" | head -1 >/dev/null
    assert_no_panic "$stderr_file"
}
