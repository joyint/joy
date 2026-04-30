#!/usr/bin/env bats
# JSON output for joy auth (--token, token add) (JOY-0138-A7, ADR-036 §1).

load setup

@test "joy auth --token --json emits session info" {
    setup_human_auth
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE" >/dev/null
    AI_TOKEN=$(joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')

    run joy auth --token "$AI_TOKEN" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
    echo "$output" | jq -e '.data.member == "ai:test@joy"' >/dev/null
    echo "$output" | jq -e '.data.delegated_by == "test@example.com"' >/dev/null
    echo "$output" | jq -e '.data.session_env | startswith("joy_s_")' >/dev/null
}

@test "joy auth token add --json emits token + member + ttl" {
    setup_human_auth
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE" >/dev/null
    run joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.member == "ai:test@joy"' >/dev/null
    echo "$output" | jq -e '.data.token | startswith("joy_t_")' >/dev/null
    echo "$output" | jq -e '.data.ttl_hours | . > 0' >/dev/null
}
