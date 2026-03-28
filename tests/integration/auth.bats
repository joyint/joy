#!/usr/bin/env bats
# Integration tests for Auth (JOY-006E).

load setup

TEST_PASSPHRASE="correct horse battery staple extra words"

# ============================================================
# joy auth init
# ============================================================

@test "joy auth init registers public key and salt" {
    joy init --name "Auth Test"
    run joy auth init --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Authentication initialized"* ]]
    # project.yaml should now have public_key and salt
    grep -q "public_key:" .joy/project.yaml
    grep -q "salt:" .joy/project.yaml
}

@test "joy auth init rejects short passphrase" {
    joy init --name "Auth Test"
    run joy auth init --passphrase "too short"
    [ "$status" -ne 0 ]
    [[ "$output" == *"passphrase too short"* ]]
}

@test "joy auth init rejects unregistered member" {
    joy init --name "Auth Test"
    git config user.email stranger@example.com
    run joy auth init --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not a registered project member"* ]]
    git config user.email test@example.com
}

@test "joy auth init rejects double initialization" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    run joy auth init --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"already has authentication"* ]]
}

# ============================================================
# joy auth (login)
# ============================================================

@test "joy auth authenticates with correct passphrase" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy deauth
    run joy auth --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Authenticated as"* ]]
}

@test "joy auth rejects wrong passphrase" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy deauth
    run joy auth --passphrase "wrong wrong wrong wrong wrong wrong"
    [ "$status" -ne 0 ]
    [[ "$output" == *"incorrect passphrase"* ]]
}

@test "joy auth rejects member without auth init" {
    joy init --name "Auth Test"
    run joy auth --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not initialized"* ]]
}

# ============================================================
# joy auth status
# ============================================================

@test "joy auth status shows active session after init" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    run joy auth status
    [ "$status" -eq 0 ]
    [[ "$output" == *"Authenticated as"* ]]
    [[ "$output" == *"Session expires"* ]]
}

@test "joy auth status shows no session after deauth" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy deauth
    run joy auth status
    [ "$status" -eq 0 ]
    [[ "$output" == *"No active session"* ]]
}

@test "joy auth status shows not initialized for new member" {
    joy init --name "Auth Test"
    run joy auth status
    [ "$status" -eq 0 ]
    [[ "$output" == *"not initialized"* ]]
}

# ============================================================
# joy deauth
# ============================================================

@test "joy deauth ends session" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    run joy deauth
    [ "$status" -eq 0 ]
    [[ "$output" == *"Session ended"* ]]
}

@test "joy deauth is safe when no session exists" {
    joy init --name "Auth Test"
    run joy deauth
    [ "$status" -eq 0 ]
}

# ============================================================
# joy auth reset
# ============================================================

@test "joy auth reset clears own auth and session" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    run joy auth reset --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Authentication reset"* ]]
    # public_key should be gone
    ! grep -q "public_key:" .joy/project.yaml
    # Can re-initialize
    run joy auth init --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
}

@test "joy auth reset rejects wrong passphrase" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    run joy auth reset --passphrase "wrong wrong wrong wrong wrong wrong"
    [ "$status" -ne 0 ]
    [[ "$output" == *"incorrect passphrase"* ]]
    # public_key should still be there
    grep -q "public_key:" .joy/project.yaml
}

@test "joy auth reset other member requires manage capability" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add dev@example.com --capabilities "implement,create"
    # Dev cannot reset others (no manage capability)
    git config user.email dev@example.com
    joy auth init --passphrase "alpha bravo charlie delta echo foxtrot"
    run joy auth reset test@example.com --passphrase "alpha bravo charlie delta echo foxtrot"
    [ "$status" -ne 0 ]
    [[ "$output" == *"manage"* ]]
    git config user.email test@example.com
}

@test "joy auth reset other member as manage user" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add dev@example.com
    # Dev initializes auth
    git config user.email dev@example.com
    joy auth init --passphrase "alpha bravo charlie delta echo foxtrot"
    git config user.email test@example.com
    # Lead (manage user) resets dev
    run joy auth reset dev@example.com --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Authentication reset for dev@example.com"* ]]
    [[ "$output" == *"re-initialize"* ]]
}

# ============================================================
# joy auth create-token
# ============================================================

@test "joy auth create-token generates token for AI member" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy
    run joy auth create-token ai:test@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"joy_t_"* ]]
    [[ "$output" == *"Delegation token for ai:test@joy"* ]]
}

@test "joy auth create-token rejects non-AI member" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add dev@example.com
    run joy auth create-token dev@example.com --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not an AI member"* ]]
}

@test "joy auth create-token rejects unregistered AI member" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    run joy auth create-token ai:unknown@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not a registered project member"* ]]
}

@test "joy auth create-token rejects wrong passphrase" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy
    run joy auth create-token ai:test@joy --passphrase "wrong wrong wrong wrong wrong wrong"
    [ "$status" -ne 0 ]
    [[ "$output" == *"incorrect passphrase"* ]]
}

@test "joy auth create-token with TTL" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy
    run joy auth create-token ai:test@joy --passphrase "$TEST_PASSPHRASE" --ttl 8
    [ "$status" -eq 0 ]
    [[ "$output" == *"expires in 8 hours"* ]]
}

# ============================================================
# Full auth flow
# ============================================================

@test "full auth flow: init -> deauth -> auth -> status -> deauth" {
    joy init --name "Auth Flow"

    # Init
    run joy auth init --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]

    # Status shows active
    run joy auth status
    [[ "$output" == *"Authenticated"* ]]

    # Deauth
    joy deauth

    # Status shows no session
    run joy auth status
    [[ "$output" == *"No active session"* ]]

    # Re-authenticate
    run joy auth --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]

    # Status shows active again
    run joy auth status
    [[ "$output" == *"Authenticated"* ]]

    # Final deauth
    joy deauth
    run joy auth status
    [[ "$output" == *"No active session"* ]]
}
