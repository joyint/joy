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
    # JOY-00ED-28: exit non-zero when no active session so scripts can gate on it.
    [ "$status" -ne 0 ]
    [[ "$output" == *"No active session"* ]]
}

@test "joy auth status shows not initialized for new member" {
    joy init --name "Auth Test"
    run joy auth status
    # JOY-00ED-28: exit non-zero when auth is not initialised.
    [ "$status" -ne 0 ]
    [[ "$output" == *"not initialized"* ]]
}

@test "joy auth status can gate shell scripts on authentication state" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    # Authenticated: status is 0, script takes the "yes" branch.
    run bash -c 'if joy auth status >/dev/null 2>&1; then echo YES; else echo NO; fi'
    [ "$status" -eq 0 ]
    [[ "$output" == *"YES"* ]]
    # After deauth: status is non-zero, script takes the "no" branch.
    joy deauth
    run bash -c 'if joy auth status >/dev/null 2>&1; then echo YES; else echo NO; fi'
    [ "$status" -eq 0 ]
    [[ "$output" == *"NO"* ]]
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
    joy project member add dev@example.com --capabilities "implement,create" --passphrase "$TEST_PASSPHRASE"
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
    joy project member add dev@example.com --passphrase "$TEST_PASSPHRASE"
    # Dev initializes auth
    git config user.email dev@example.com
    joy auth init --passphrase "alpha bravo charlie delta echo foxtrot"
    git config user.email test@example.com
    # Re-authenticate as lead (dev's auth init overwrote the session)
    joy auth --passphrase "$TEST_PASSPHRASE"
    # Lead (manage user) resets dev
    run joy auth reset dev@example.com --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Authentication reset for dev@example.com"* ]]
    [[ "$output" == *"re-initialize"* ]]
}

# ============================================================
# joy auth token add / rm
# ============================================================

@test "joy auth token add generates token for AI member" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    run joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"joy_t_"* ]]
    [[ "$output" == *"Delegation token for ai:test@joy"* ]]
}

@test "joy auth token add rejects non-AI member" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add dev@example.com --passphrase "$TEST_PASSPHRASE"
    run joy auth token add dev@example.com --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not an AI member"* ]]
}

@test "joy auth token add rejects unregistered AI member" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    run joy auth token add ai:unknown@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not a registered project member"* ]]
}

@test "joy auth token add rejects wrong passphrase" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    run joy auth token add ai:test@joy --passphrase "wrong wrong wrong wrong wrong wrong"
    [ "$status" -ne 0 ]
    [[ "$output" == *"incorrect passphrase"* ]]
}

@test "joy auth token add with TTL" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    run joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE" --ttl 8
    [ "$status" -eq 0 ]
    [[ "$output" == *"expires in 8 hours"* ]]
}

@test "joy auth token rm revokes token" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    run joy auth token rm ai:test@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Delegation for ai:test@joy revoked"* ]]
}

@test "joy auth token rm rejects when no token exists" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    run joy auth token rm ai:test@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"No delegation registered"* ]]
}

# ============================================================
# Single-use tokens and 2h TTL (ADR-033 / JOY-00EA-45)
# ============================================================

@test "delegation token is multi-use within its TTL" {
    # ADR-034 relaxes ADR-033 §3: the same token may be redeemed multiple
    # times within its TTL, each redemption producing an independent session.
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    TOKEN=$(joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')
    [ -n "$TOKEN" ]
    run joy auth --token "$TOKEN"
    [ "$status" -eq 0 ]
    # Second redemption of the same token also succeeds.
    run joy auth --token "$TOKEN"
    [ "$status" -eq 0 ]
}

@test "delegation token announces 24h default TTL" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    run joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"expires in 24 hours"* ]]
}

# ============================================================
# Session isolation per member (JOY-008A)
# ============================================================

@test "two members can have independent sessions" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add dev@example.com --passphrase "$TEST_PASSPHRASE"
    # Dev initializes auth
    git config user.email dev@example.com
    joy auth init --passphrase "alpha bravo charlie delta echo foxtrot"
    # Both should have active sessions
    run joy auth status
    [ "$status" -eq 0 ]
    [[ "$output" == *"Authenticated as dev@example.com"* ]]
    # Switch back to lead
    git config user.email test@example.com
    run joy auth status
    [ "$status" -eq 0 ]
    [[ "$output" == *"Authenticated as test@example.com"* ]]
}

@test "deauth only removes own session" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add dev@example.com --passphrase "$TEST_PASSPHRASE"
    git config user.email dev@example.com
    joy auth init --passphrase "alpha bravo charlie delta echo foxtrot"
    # Dev deauths
    joy deauth
    run joy auth status
    [[ "$output" == *"No active session"* ]]
    # Lead still has session
    git config user.email test@example.com
    run joy auth status
    [[ "$output" == *"Authenticated as test@example.com"* ]]
}

# ============================================================
# joy ai reset cleans up auth (JOY-0089)
# ============================================================

@test "joy ai reset removes AI member and its auth data" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    # Manually create tool directory and register AI member
    mkdir -p .claude
    echo "# test" > .claude/CLAUDE.md
    joy project member add ai:claude@joy --passphrase "$TEST_PASSPHRASE"
    # Create a delegation token and authenticate as AI
    TOKEN=$(joy auth token add ai:claude@joy --passphrase "$TEST_PASSPHRASE" | sed -n 's/^  \(joy_t_.*\)/\1/p')
    joy auth --token "$TOKEN"
    # Verify AI member exists with public_key (set by token auth)
    grep -q "ai:claude@joy" .joy/project.yaml
    grep -q "public_key" .joy/project.yaml
    # Reset the AI tool
    joy ai reset --tool claude --force
    # AI member should be removed from project.yaml
    ! grep -q "ai:claude@joy" .joy/project.yaml
}

@test "joy ai reset removes all AI members when resetting all tools" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:claude@joy --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:qwen@joy --passphrase "$TEST_PASSPHRASE"
    # Verify both exist
    grep -q "ai:claude@joy" .joy/project.yaml
    grep -q "ai:qwen@joy" .joy/project.yaml
    # Create tool directories so reset has something to remove
    mkdir -p .claude .qwen
    touch .claude/CLAUDE.md .qwen/QWEN.md
    # Reset all
    joy ai reset --force
    # Both AI members should be removed
    ! grep -q "ai:claude@joy" .joy/project.yaml
    ! grep -q "ai:qwen@joy" .joy/project.yaml
}

@test "joy ai reset removes .joy/ai/ directory" {
    joy init --name "Auth Test"
    joy ai init </dev/null 2>/dev/null || true
    # Verify .joy/ai/ exists
    [ -d ".joy/ai" ]
    # Create tool directories so reset has something to remove
    mkdir -p .claude
    touch .claude/CLAUDE.md
    joy project member add ai:claude@joy 2>/dev/null --passphrase "$TEST_PASSPHRASE" || true
    # Reset all
    joy ai reset --force
    # .joy/ai/ should be removed
    [ ! -d ".joy/ai" ]
}

@test "joy ai reset preserves .joy/ai/jobs/ when non-empty" {
    joy init --name "Auth Test"
    joy ai init </dev/null 2>/dev/null || true
    # Put content in jobs/
    mkdir -p .joy/ai/jobs
    echo "test-job" > .joy/ai/jobs/job-001.yaml
    # Create tool directories so reset has something to remove
    mkdir -p .claude
    touch .claude/CLAUDE.md
    joy project member add ai:claude@joy 2>/dev/null --passphrase "$TEST_PASSPHRASE" || true
    # Reset all
    joy ai reset --force
    # jobs/ should be preserved
    [ -f ".joy/ai/jobs/job-001.yaml" ]
    # but other ai/ contents should be gone
    [ ! -d ".joy/ai/agents" ]
}

# ============================================================
# cross-project session isolation (JOY-00CB)
# ============================================================

@test "AI session token rejected in different project" {
    # Create project A
    mkdir -p project_a && cd project_a
    git init --quiet
    git config user.email "test@example.com"
    git config user.name "Test User"
    joy init --name "Project A" --acronym PRJA
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:claude@joy --passphrase "$TEST_PASSPHRASE"
    # Create AI token scoped to project A
    TOKEN=$(joy auth token add ai:claude@joy --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')
    eval $(joy auth --token "$TOKEN")
    SESS="$JOY_SESSION"
    # Verify it works in project A
    run env JOY_SESSION="$SESS" joy add task "Test in A"
    [ "$status" -eq 0 ]
    # Create project B
    cd "$TEST_DIR"
    mkdir -p project_b && cd project_b
    git init --quiet
    git config user.email "test@example.com"
    git config user.name "Test User"
    joy init --name "Project B" --acronym PRJB
    # Project B does not initialize auth. Register ai:claude@joy as a
    # member via a direct yaml edit so we can exercise the cross-project
    # session isolation check without triggering the attestation-signing
    # flow (which requires an authenticated manage member).
    cat >> .joy/project.yaml <<'YAML'
  ai:claude@joy:
    capabilities: all
YAML
    # Use project A's session in project B - must be rejected
    run env JOY_SESSION="$SESS" joy add task "Test in B"
    [ "$status" -ne 0 ]
}

# ============================================================
# write_yaml_preserve (JOY-008B)
# ============================================================

@test "project.yaml extra fields survive auth init" {
    joy init --name "Auth Test"
    # Add a custom field not in the Project struct
    echo 'release:' >> .joy/project.yaml
    echo '  version-files:' >> .joy/project.yaml
    echo '  - path: Cargo.toml' >> .joy/project.yaml
    echo '    key: package.version' >> .joy/project.yaml
    # Auth init modifies project.yaml (adds public_key, salt)
    joy auth init --passphrase "$TEST_PASSPHRASE"
    # The release config must survive
    grep -q "version-files" .joy/project.yaml
    grep -q "Cargo.toml" .joy/project.yaml
}

@test "project.yaml extra fields survive member add" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    echo 'custom_field: preserved' >> .joy/project.yaml
    joy project member add dev@example.com --passphrase "$TEST_PASSPHRASE"
    grep -q "custom_field: preserved" .joy/project.yaml
}

@test "project.yaml extra fields survive auth reset" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    echo 'custom_field: preserved' >> .joy/project.yaml
    joy auth reset --passphrase "$TEST_PASSPHRASE"
    grep -q "custom_field: preserved" .joy/project.yaml
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

# ============================================================
# joy auth passphrase (JOY-0073)
# ============================================================

@test "joy auth passphrase rotates keypair preserving attestation" {
    joy init --name "Passphrase Test" --acronym PT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    OTP=$(joy project member add alice@example.com --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^[[:space:]]*One-time password:[[:space:]]*\([A-Za-z0-9-]*\).*$/\1/p' | head -1)
    git config user.email alice@example.com
    joy auth --otp "$OTP" --passphrase "alpha bravo charlie delta echo foxtrot"

    OLD_PUB=$(grep -A3 "^  alice@example.com:" .joy/project.yaml | grep "public_key:" | awk '{print $NF}')
    OLD_ATT=$(grep -A20 "^  alice@example.com:" .joy/project.yaml | grep "signature:" | head -1)

    run joy auth passphrase \
        --passphrase "alpha bravo charlie delta echo foxtrot" \
        --new-passphrase "kilo lima mike november oscar papa"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Passphrase changed"* ]]

    # public_key rotated, attestation preserved.
    NEW_PUB=$(grep -A3 "^  alice@example.com:" .joy/project.yaml | grep "public_key:" | awk '{print $NF}')
    NEW_ATT=$(grep -A20 "^  alice@example.com:" .joy/project.yaml | grep "signature:" | head -1)
    [ "$OLD_PUB" != "$NEW_PUB" ]
    [ "$OLD_ATT" = "$NEW_ATT" ]

    # New passphrase works; old passphrase does not.
    run joy auth --passphrase "kilo lima mike november oscar papa"
    [ "$status" -eq 0 ]
    joy deauth
    run joy auth --passphrase "alpha bravo charlie delta echo foxtrot"
    [ "$status" -ne 0 ]
}

@test "joy auth passphrase rejects wrong current passphrase" {
    joy init --name "Passphrase Test" --acronym PT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    run joy auth passphrase \
        --passphrase "wrong wrong wrong wrong wrong wrong" \
        --new-passphrase "kilo lima mike november oscar papa"
    [ "$status" -ne 0 ]
    [[ "$output" == *"incorrect passphrase"* ]]
}

@test "joy auth passphrase rejects identical new passphrase" {
    joy init --name "Passphrase Test" --acronym PT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    run joy auth passphrase \
        --passphrase "$TEST_PASSPHRASE" \
        --new-passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"must differ"* ]]
}

# ============================================================
# project set acronym migrates delegation directory (JOY-00F7-91)
# ============================================================

@test "project set acronym migrates local delegation directory" {
    joy init --name "Rename Test" --acronym OLDACR
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    # First-time token issuance creates the local delegation key file.
    joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE" >/dev/null
    [ -f "$XDG_STATE_HOME/joy/delegations/OLDACR/ai_test_joy.key" ]

    run joy project set acronym NEWACR
    [ "$status" -eq 0 ]
    [[ "$output" == *"Local delegation keys have been migrated"* ]]

    # Key migrated to new path, old path gone.
    [ -f "$XDG_STATE_HOME/joy/delegations/NEWACR/ai_test_joy.key" ]
    [ ! -d "$XDG_STATE_HOME/joy/delegations/OLDACR" ]

    # Session was scoped to the old acronym. After re-auth under the new
    # acronym, token issuance reuses the migrated private key without
    # generating a new keypair (no project.yaml write).
    joy auth --passphrase "$TEST_PASSPHRASE"
    run joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"joy_t_"* ]]
}

@test "project set acronym refuses when target delegation directory exists" {
    joy init --name "Rename Test" --acronym OLDACR
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE" >/dev/null
    # Pre-populate a conflicting target directory.
    mkdir -p "$XDG_STATE_HOME/joy/delegations/NEWACR"
    echo "unrelated" > "$XDG_STATE_HOME/joy/delegations/NEWACR/placeholder"

    run joy project set acronym NEWACR
    [ "$status" -ne 0 ]
    [[ "$output" == *"already exists"* ]]

    # Old directory untouched, project.yaml acronym still OLDACR.
    [ -f "$XDG_STATE_HOME/joy/delegations/OLDACR/ai_test_joy.key" ]
    run joy project get acronym
    [ "$output" = "OLDACR" ]
}

@test "joy ai rotate replaces delegation keypair on working state" {
    joy init --name "Rotate Test" --acronym RT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    # Initial delegation.
    OLD_TOKEN=$(joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')
    OLD_KEY_HASH=$(sha256sum "$XDG_STATE_HOME/joy/delegations/RT/ai_test_joy.key" | cut -d' ' -f1)
    OLD_PUB=$(grep -A2 "ai:test@joy:" .joy/project.yaml | grep delegation_key | sed 's/.*: //')

    run joy ai rotate ai:test@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Rotated delegation"* ]]
    [[ "$output" == *"invalidated"* ]]

    # Local private key file replaced with a different one.
    NEW_KEY_HASH=$(sha256sum "$XDG_STATE_HOME/joy/delegations/RT/ai_test_joy.key" | cut -d' ' -f1)
    [ "$OLD_KEY_HASH" != "$NEW_KEY_HASH" ]

    # project.yaml has new delegation_key plus a rotated timestamp.
    NEW_PUB=$(grep -A2 "ai:test@joy:" .joy/project.yaml | grep delegation_key | sed 's/.*: //')
    [ "$OLD_PUB" != "$NEW_PUB" ]
    grep -q "rotated:" .joy/project.yaml

    # A newly issued token works; the old token is invalidated.
    NEW_TOKEN=$(joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')
    run joy auth --token "$NEW_TOKEN"
    [ "$status" -eq 0 ]
    run joy auth --token "$OLD_TOKEN"
    [ "$status" -ne 0 ]
}

@test "joy ai rotate recovers when local private key is missing" {
    joy init --name "Rotate Test" --acronym RT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE" >/dev/null
    # Simulate the (Some pub, None priv) desync: remove the local key.
    rm "$XDG_STATE_HOME/joy/delegations/RT/ai_test_joy.key"

    # Without rotate, token add bails out with the desync message.
    run joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"missing on this machine"* ]]

    # Rotate re-establishes both sides.
    run joy ai rotate ai:test@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [ -f "$XDG_STATE_HOME/joy/delegations/RT/ai_test_joy.key" ]
    run joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
}

@test "joy ai rotate refuses when no delegation entry exists" {
    joy init --name "Rotate Test" --acronym RT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    # No token add -> no ai_delegations entry in project.yaml.

    run joy ai rotate ai:test@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"No delegation"* ]]
    [[ "$output" == *"joy auth token add"* ]]
}

@test "joy ai rotate rejects non-AI member" {
    joy init --name "Rotate Test" --acronym RT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add dev@example.com --passphrase "$TEST_PASSPHRASE"
    run joy ai rotate dev@example.com --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not an AI member"* ]]
}

@test "joy ai rotate rejects wrong passphrase" {
    joy init --name "Rotate Test" --acronym RT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE" >/dev/null
    run joy ai rotate ai:test@joy --passphrase "wrong wrong wrong wrong wrong wrong"
    [ "$status" -ne 0 ]
    [[ "$output" == *"incorrect passphrase"* ]]
}

@test "project set acronym is no-op when no delegation directory exists" {
    joy init --name "Rename Test" --acronym OLDACR
    joy auth init --passphrase "$TEST_PASSPHRASE"
    # No delegation issued, no key directory on disk yet.
    [ ! -d "$XDG_STATE_HOME/joy/delegations/OLDACR" ]

    run joy project set acronym NEWACR
    [ "$status" -eq 0 ]
    [ ! -d "$XDG_STATE_HOME/joy/delegations/OLDACR" ]
    [ ! -d "$XDG_STATE_HOME/joy/delegations/NEWACR" ]
}
