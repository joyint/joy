#!/usr/bin/env bats
# JSON output for joy ai update --check (JOY-012D-F4, ADR-036 §1).

load setup

@test "joy ai update --check --json emits tool list" {
    setup_human_auth
    run joy ai update --check --json
    [ "$status" -eq 0 ] || [ "$status" -eq 2 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
    echo "$output" | jq -e '.data | has("tools")' >/dev/null
    echo "$output" | jq -e '.data.check == true' >/dev/null
    echo "$output" | jq -e '.data.tools | length >= 4' >/dev/null
    echo "$output" | jq -e '[.data.tools[].id] | contains(["claude","qwen","vibe","copilot"])' >/dev/null
}

@test "joy ai update (no --check) --json reports tool changes" {
    setup_human_auth
    run joy ai update --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.check == false' >/dev/null
    echo "$output" | jq -e '.data.has_issues == false' >/dev/null
}
