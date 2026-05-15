#!/usr/bin/env bats
# joy project member add --with-token: combined registration + delegation
# token issuance for AI members (JOY-0185-66).

load setup

@test "--with-token issues a delegation token in one go for ai: members" {
    setup_human_auth

    run joy project member add ai:claude@joy --with-token \
        --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Added member ai:claude@joy"* ]]

    # Token printed on its own line so the operator can copy/paste it.
    local token
    token=$(echo "$output" | grep '^joy_t_' | head -n 1)
    [[ "$token" == joy_t_* ]]

    # No second `auth token add` step is needed: the AI can redeem the
    # token directly.
    local redeem
    redeem=$(joy auth --token "$token" --json)
    local member session
    member=$(echo "$redeem" | jq -r '.data.member')
    session=$(echo "$redeem" | jq -r '.data.session_env')
    [ "$member" = "ai:claude@joy" ]
    [[ "$session" == joy_s_* ]]
}

@test "--with-token JSON output includes member, token, ttl_hours" {
    setup_human_auth

    run joy project member add ai:gemini@joy --with-token --json \
        --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    local member token ttl
    member=$(echo "$output" | jq -r '.data.member')
    token=$(echo "$output" | jq -r '.data.token')
    ttl=$(echo "$output" | jq -r '.data.ttl_hours')
    [ "$member" = "ai:gemini@joy" ]
    [[ "$token" == joy_t_* ]]
    [ "$ttl" = "24" ]
}

@test "--with-token is a no-op for human members (no token printed)" {
    setup_human_auth

    run joy project member add dev@example.com --with-token \
        --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Added member dev@example.com"* ]]
    [[ "$output" != *"joy_t_"* ]]
    # Human flow still surfaces the OTP.
    [[ "$output" == *"One-time password:"* ]]
}

@test "without --with-token the AI add does NOT issue a token" {
    setup_human_auth

    run joy project member add ai:noauto@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" != *"joy_t_"* ]]
}
