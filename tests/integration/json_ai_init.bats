#!/usr/bin/env bats
# JSON output for joy ai init (JOY-0137-42, ADR-036 §1).

load setup

setup_fake_ai_tools() {
    BIN_DIR="$TEST_DIR/fake-bin"
    mkdir -p "$BIN_DIR"
    for cmd in claude copilot; do
        printf '#!/bin/sh\nexit 0\n' > "$BIN_DIR/$cmd"
        chmod +x "$BIN_DIR/$cmd"
    done
    PATH="$BIN_DIR:$PATH"
}

@test "joy ai init --json emits configured tools list" {
    setup_human_auth
    setup_fake_ai_tools

    run joy ai init --passphrase "$TEST_PASSPHRASE" --json </dev/null
    [ "$status" -eq 0 ]
    # Must contain exactly one JSON object (envelope)
    last_line=$(echo "$output" | grep -E '^\{' | tail -1)
    echo "$last_line" | jq -e '.version == 1' >/dev/null
    echo "$last_line" | jq -e '.data | has("configured_tools")' >/dev/null
    echo "$last_line" | jq -e '[.data.configured_tools[]] | contains(["claude","copilot"])' >/dev/null
}
