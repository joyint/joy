#!/usr/bin/env bats
# joy project member add: next-steps hint for AI members and discoverability
# from `joy ai --help` (JOY-0183-04).

load setup

@test "joy project member add prints next-steps hint for ai: members" {
    setup_human_auth
    run joy project member add copilot-chat --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Added member copilot-chat"* ]]
    [[ "$output" == *"Next steps:"* ]]
    [[ "$output" == *"joy auth token add copilot-chat"* ]]
    [[ "$output" == *"joy auth --token <TOKEN> --json"* ]]
    [[ "$output" == *"session_env"* ]]
    [[ "$output" == *"joy ai tutorial"* ]]
    [[ "$output" == *"--with-token"* ]]
}

@test "joy project member add still prints OTP flow for human members" {
    setup_human_auth
    run joy project member add dev@example.com --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Added member dev@example.com"* ]]
    [[ "$output" == *"One-time password:"* ]]
    [[ "$output" != *"session_env"* ]]
}

@test "joy ai --help suggests project member add for undetected AI tools" {
    setup_human_auth
    run joy ai --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"chat-only"* ]]
    [[ "$output" == *"joy project member add <name>"* ]]
}

# --- end-to-end: the workflow that the next-steps hint guides users through ---

@test "end-to-end: add generic AI member, issue token, redeem, act as AI" {
    setup_human_auth

    # Step 1: register a generic AI member (e.g. a chat-only tool).
    run joy project member add copilot-chat \
        --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    members_grep -q "^name: copilot-chat"

    # Step 2: issue a delegation token (as the next-steps hint instructs).
    # The command prints the token wrapped in double quotes; strip them
    # for the bare value used on the next line.
    local token
    token=$(joy auth token add copilot-chat --passphrase "$TEST_PASSPHRASE" | tr -d '"')
    [[ "$token" == joy_t_* ]]

    # Step 3: AI redeems the token via --json and learns its identity + auth.
    local redeem
    redeem=$(joy auth --token "$token" --json)
    local member session
    member=$(echo "$redeem" | jq -r '.data.member')
    session=$(echo "$redeem" | jq -r '.data.session_env')
    [ "$member" = "copilot-chat" ]
    [[ "$session" == joy_s_* ]]

    # Step 4: use --session to act as the AI on a write command.
    run joy --session "$session" add task "ai-created item"
    [ "$status" -eq 0 ]

    # Verify the created item is recorded with the AI as author in the event log.
    run grep -l "copilot-chat" .joy/logs/*.log
    [ "$status" -eq 0 ]
}

@test "bogus --session value does not authenticate as the AI" {
    setup_human_auth
    setup_ai_session testai
    local good="$JOY_SESSION"
    unset JOY_SESSION

    # The good session works.
    run joy --session "$good" auth status
    [ "$status" -eq 0 ]
    [[ "$output" == *"Member:"*"testai"* ]]

    # A garbage value must not yield the AI session, and it must not
    # yield the session of the person who signed in here either: a
    # process that carries a session value never falls back to a human
    # session. It is told that the value is dead, and it stands as
    # unproven as git config.
    run joy --session "joy_s_bogus" auth status
    [[ "$output" == *"JOY_SESSION is no longer valid"* ]]
    [[ "$output" == *"No active session"* ]]
    member_line=$(echo "$output" | grep -E "^\s*Member:" | head -n 1)
    [[ "$member_line" != *"testai"* ]]
}

@test "joy project member add fails when the AI member already exists" {
    setup_human_auth
    joy project member add dup --passphrase "$TEST_PASSPHRASE" >/dev/null
    run joy project member add dup --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"already exists"* ]]
}

@test "joy auth --token rejects a malformed token" {
    setup_human_auth
    run joy auth --token "joy_t_not-a-real-token-payload"
    [ "$status" -ne 0 ]
}
