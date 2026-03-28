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
