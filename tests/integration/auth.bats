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
    # project.yaml should now have verify_key and kdf_nonce
    grep -q "verify_key:" .joy/project.yaml
    grep -q "kdf_nonce:" .joy/project.yaml
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
    [[ "$output" == *"Auth Status"* ]]
    [[ "$output" == *"Your session"* ]]
    [[ "$output" == *"Member:"* ]]
    [[ "$output" == *"Expires:"* ]]
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
    # verify_key should be gone
    ! grep -q "verify_key:" .joy/project.yaml
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
    # verify_key should still be there
    grep -q "verify_key:" .joy/project.yaml
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

@test "joy auth delegation rotate replaces the keypair" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    local before
    before=$(grep delegation_verifier .joy/project.yaml | head -1)
    run joy auth delegation rotate ai:test@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Rotated delegation for ai:test@joy"* ]]
    local after
    after=$(grep delegation_verifier .joy/project.yaml | head -1)
    [ "$before" != "$after" ]
}

@test "joy auth delegation rotate rejects when no delegation exists" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    run joy auth delegation rotate ai:test@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"No delegation"* ]]
}

@test "joy auth delegation ls lists registered delegations" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    run joy auth delegation ls
    [ "$status" -eq 0 ]
    [[ "$output" == *"ai:test@joy"* ]]
    [[ "$output" == *"OPERATOR"* ]]
}

@test "joy auth delegation ls reports empty backlog cleanly" {
    joy init --name "Auth Test"
    joy auth init --passphrase "$TEST_PASSPHRASE"
    run joy auth delegation ls
    [ "$status" -eq 0 ]
    [[ "$output" == *"No AI delegations registered"* ]]
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
    [[ "$output" == *"dev@example.com"* ]]
    [[ "$output" == *"Expires:"* ]]
    # Switch back to lead
    git config user.email test@example.com
    run joy auth status
    [ "$status" -eq 0 ]
    [[ "$output" == *"test@example.com"* ]]
    [[ "$output" == *"Expires:"* ]]
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
    [[ "$output" == *"test@example.com"* ]]
    [[ "$output" == *"Expires:"* ]]
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
    # Verify AI member exists with verify_key (set by token auth)
    grep -q "ai:claude@joy" .joy/project.yaml
    grep -q "verify_key" .joy/project.yaml
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
    # Auth init modifies project.yaml (adds verify_key, kdf_nonce)
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
    [[ "$output" == *"Auth Status"* ]]
    [[ "$output" == *"Member:"* ]]

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
    [[ "$output" == *"Auth Status"* ]]
    [[ "$output" == *"Member:"* ]]

    # Final deauth
    joy deauth
    run joy auth status
    [[ "$output" == *"No active session"* ]]
}

# ============================================================
# joy auth passphrase (JOY-0073)
# ============================================================

@test "joy auth passphrase preserves keypair and attestation (ADR-039)" {
    joy init --name "Passphrase Test" --acronym PT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    OTP=$(joy project member add alice@example.com --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^[[:space:]]*One-time password:[[:space:]]*\([A-Za-z0-9-]*\).*$/\1/p' | head -1)
    git config user.email alice@example.com
    joy auth --otp "$OTP" --passphrase "alpha bravo charlie delta echo foxtrot"

    # alice's capability block is now multi-line (defaults exclude
    # manage/delete, so nine capability keys each render on their own
    # line). Use a generous -A range so the follow-up greps still reach
    # verify_key and signature.
    OLD_PUB=$(grep -A40 "^  alice@example.com:" .joy/project.yaml | grep "verify_key:" | awk '{print $NF}')
    OLD_ATT=$(grep -A40 "^  alice@example.com:" .joy/project.yaml | grep "signature:" | head -1)
    OLD_WRAP=$(grep -A40 "^  alice@example.com:" .joy/project.yaml | grep "seed_wrap_passphrase:" | awk '{print $NF}')

    run joy auth passphrase \
        --passphrase "alpha bravo charlie delta echo foxtrot" \
        --new-passphrase "kilo lima mike november oscar papa"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Passphrase changed"* ]]

    # verify_key preserved (ADR-039 wrapped-seed model: keypair derives
    # from a stable seed). seed_wrap_passphrase rotates. Attestation is
    # untouched.
    NEW_PUB=$(grep -A40 "^  alice@example.com:" .joy/project.yaml | grep "verify_key:" | awk '{print $NF}')
    NEW_ATT=$(grep -A40 "^  alice@example.com:" .joy/project.yaml | grep "signature:" | head -1)
    NEW_WRAP=$(grep -A40 "^  alice@example.com:" .joy/project.yaml | grep "seed_wrap_passphrase:" | awk '{print $NF}')
    [ "$OLD_PUB" = "$NEW_PUB" ]
    [ "$OLD_ATT" = "$NEW_ATT" ]
    [ "$OLD_WRAP" != "$NEW_WRAP" ]

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

@test "joy ai rotate replaces delegation keypair on working state" {
    joy init --name "Rotate Test" --acronym RT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    # Initial delegation.
    OLD_TOKEN=$(joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE" \
        | grep -o 'joy_t_[A-Za-z0-9+/=]*' | head -1)
    OLD_PUB=$(grep -A2 "ai:test@joy:" .joy/project.yaml | grep delegation_verifier | sed 's/.*: //')

    run joy ai rotate ai:test@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Rotated delegation"* ]]
    [[ "$output" == *"invalidated"* ]]

    # project.yaml has new delegation_verifier plus a rotated timestamp.
    NEW_PUB=$(grep -A2 "ai:test@joy:" .joy/project.yaml | grep delegation_verifier | sed 's/.*: //')
    [ "$OLD_PUB" != "$NEW_PUB" ]
    grep -q "rotated:" .joy/project.yaml

    # A newly issued token works; the old token is invalidated.
    NEW_TOKEN=$(joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE" \
        | grep -o 'joy_t_[A-Za-z0-9+/=]*' | head -1)
    run joy auth --token "$NEW_TOKEN"
    [ "$status" -eq 0 ]
    run joy auth --token "$OLD_TOKEN"
    [ "$status" -ne 0 ]
}

@test "joy auth token add bails on legacy delegation without salt" {
    joy init --name "Rotate Test" --acronym RT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE" >/dev/null
    # Simulate a legacy entry by stripping delegation_salt from project.yaml.
    sed_inplace '/^        delegation_salt:/d' .joy/project.yaml

    run joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"Cannot issue a new token"* ]]
    [[ "$output" == *"joy auth delegation rotate"* ]]

    # Rotation writes a fresh salt and unblocks subsequent issuance.
    run joy ai rotate ai:test@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
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

@test "joy auth init displays recovery key once (ADR-039)" {
    joy init --name "Recovery Test" --acronym RT
    run joy auth init --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"RECOVERY KEY"* ]]
    [[ "$output" == *"joy_r_"* ]]

    # Both wraps land in project.yaml.
    grep -q "seed_wrap_passphrase:" .joy/project.yaml
    grep -q "seed_wrap_recovery:" .joy/project.yaml
}

@test "joy auth recover --recovery-key resets passphrase preserving keypair (ADR-039)" {
    joy init --name "Recovery Test" --acronym RT
    OUT=$(joy auth init --passphrase "$TEST_PASSPHRASE")
    REC=$(echo "$OUT" | sed -n 's/^.*\(joy_r_[0-9a-f]\{64\}\).*$/\1/p' | head -1)
    [ -n "$REC" ]

    OLD_PUB=$(grep "verify_key:" .joy/project.yaml | head -1 | awk '{print $NF}')

    run joy auth recover --recovery-key \
        --recovery "$REC" \
        --new-passphrase "kilo lima mike november oscar papa"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Recovery successful"* ]]

    # Keypair preserved (verify_key unchanged).
    NEW_PUB=$(grep "verify_key:" .joy/project.yaml | head -1 | awk '{print $NF}')
    [ "$OLD_PUB" = "$NEW_PUB" ]

    # New passphrase works; old does not.
    run joy auth --passphrase "kilo lima mike november oscar papa"
    [ "$status" -eq 0 ]
    joy deauth
    run joy auth --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
}

@test "joy auth recover --regenerate-key rotates recovery wrap (ADR-039)" {
    joy init --name "Recovery Test" --acronym RT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    OLD_REC_WRAP=$(grep "seed_wrap_recovery:" .joy/project.yaml | awk '{print $NF}')
    OLD_PASS_WRAP=$(grep "seed_wrap_passphrase:" .joy/project.yaml | awk '{print $NF}')

    run joy auth recover --regenerate-key --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Recovery key rotated"* ]]
    [[ "$output" == *"NEW RECOVERY KEY"* ]]

    NEW_REC_WRAP=$(grep "seed_wrap_recovery:" .joy/project.yaml | awk '{print $NF}')
    NEW_PASS_WRAP=$(grep "seed_wrap_passphrase:" .joy/project.yaml | awk '{print $NF}')
    [ "$OLD_REC_WRAP" != "$NEW_REC_WRAP" ]
    [ "$OLD_PASS_WRAP" = "$NEW_PASS_WRAP" ]

    # Existing passphrase still works.
    run joy auth --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
}

# Lazy-migration end-to-end is exercised by the joy-core unit tests
# (`auth::seed::tests`), where a legacy state with `verify_key` derived
# directly from passphrase+kdf_nonce is straightforward to construct.
# A bats-only legacy fixture would have to recreate Argon2id parameters
# and yaml byte layout manually, so that path stays in Rust tests.

@test "joy crypt status shows empty config on fresh project" {
    joy init --name "Crypt Test" --acronym CT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    run joy crypt status
    [ "$status" -eq 0 ]
    [[ "$output" == *"zones registered:  0"* ]]
    [[ "$output" == *"items in any zone: 0"* ]]
    [[ "$output" == *"No encryption configured"* ]]
}

@test "joy crypt add encrypts the item file in place (ADR-040)" {
    joy init --name "Crypt Test" --acronym CT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy add task "Sensitive item" >/dev/null

    ITEM_FILE=$(ls .joy/items/CT-*.yaml | head -1)
    ID=$(basename "$ITEM_FILE" | sed -E 's/^(CT-[0-9A-Fa-f]+(-[0-9A-Fa-f]+)?)-.*/\1/')

    # Before: file is plaintext yaml.
    head -c 8 "$ITEM_FILE" | grep -qv "JOYCRYPT"

    run joy crypt add "$ID" --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Added"* ]]

    # After: item file is a JOYCRYPT blob (encrypt-on-disk).
    HEAD8=$(head -c 8 "$ITEM_FILE")
    [ "$HEAD8" = "JOYCRYPT" ]

    # Status walks metadata only - no passphrase needed.
    run joy crypt status
    [ "$status" -eq 0 ]
    [[ "$output" == *"items in any zone: 1"* ]]
}

@test "joy crypt add encrypts a free file in place" {
    joy init --name "Crypt Test" --acronym CT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    mkdir -p data/customer-x
    echo "secret payload" > data/customer-x/notes.txt

    run joy crypt add "data/customer-x/" --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]

    # Path registered in project.yaml.
    grep -q "data/customer-x/" .joy/project.yaml

    # File on disk is now ciphertext.
    HEAD8=$(head -c 8 data/customer-x/notes.txt)
    [ "$HEAD8" = "JOYCRYPT" ]

    # ls shows the path.
    run joy crypt ls
    [ "$status" -eq 0 ]
    [[ "$output" == *"data/customer-x/"* ]]
}

@test "joy crypt grant wraps the zone key for another member (X25519, JOY-0157-86)" {
    joy init --name "Crypt Test" --acronym CT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    OTP=$(joy project member add bob@example.com --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^[[:space:]]*One-time password:[[:space:]]*\([A-Za-z0-9-]*\).*$/\1/p' | head -1)
    [ -n "$OTP" ]

    # Bob redeems the OTP so he has a verify_key registered for ECDH.
    git config user.email bob@example.com
    joy auth --otp "$OTP" --passphrase "alpha bravo charlie delta echo foxtrot" \
        | grep -q "Authentication initialized"
    git config user.email test@example.com

    # Founder seeds the default zone via a real path, then grants Bob.
    mkdir -p secret
    echo "x" > secret/file.txt
    joy crypt add "secret/" --passphrase "$TEST_PASSPHRASE" >/dev/null
    run joy crypt grant bob@example.com --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Granted"* ]]

    # Bob's member entry now has a wrap; founder still has theirs.
    grep -A 25 "^  bob@example.com:" .joy/project.yaml | grep -q "default:"
    grep -A 25 "^  test@example.com:" .joy/project.yaml | grep -q "default:"

    # zone ls reports the default zone.
    run joy crypt zone ls
    [[ "$output" == *"default"* ]]
}

@test "joy crypt zone ls reports paths/items/members per zone" {
    joy init --name "Crypt Test" --acronym CT
    joy auth init --passphrase "$TEST_PASSPHRASE"

    # Default zone via real path; named zone via --zone.
    mkdir -p secret data/customer-x
    echo "a" > secret/a.txt
    echo "b" > data/customer-x/b.txt
    joy crypt add "secret/" --passphrase "$TEST_PASSPHRASE" >/dev/null
    joy crypt add "data/customer-x/" --zone customer-x --passphrase "$TEST_PASSPHRASE" >/dev/null

    run joy crypt zone ls
    [ "$status" -eq 0 ]
    [[ "$output" == *"default"* ]]
    [[ "$output" == *"customer-x"* ]]
}

@test "joy crypt zone rm refuses non-empty zones" {
    joy init --name "Crypt Test" --acronym CT
    joy auth init --passphrase "$TEST_PASSPHRASE"

    mkdir -p secret
    echo "x" > secret/file.txt
    joy crypt add "secret/" --passphrase "$TEST_PASSPHRASE" >/dev/null

    run joy crypt zone rm default
    [ "$status" -ne 0 ]
    [[ "$output" == *"not empty"* ]]
}

@test "joy crypt add --all encrypts every item under the zone" {
    joy init --name "Crypt Test" --acronym CT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy add task "Item one" >/dev/null
    joy add task "Item two" >/dev/null

    run joy crypt add --all --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"item(s) encrypted"* ]]

    run joy crypt status
    [[ "$output" == *"items in any zone: 2"* ]]

    # Each item file is now a JOYCRYPT blob.
    for f in .joy/items/CT-*.yaml; do
        HEAD8=$(head -c 8 "$f")
        [ "$HEAD8" = "JOYCRYPT" ]
    done
}

@test "joy crypt rm --all decrypts every item back to plaintext" {
    joy init --name "Crypt Test" --acronym CT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy add task "Item one" >/dev/null

    joy crypt add --all --passphrase "$TEST_PASSPHRASE" >/dev/null

    run joy crypt rm --all --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Decrypted"* ]]

    run joy crypt status
    [[ "$output" == *"items in any zone: 0"* ]]

    # Files are plaintext yaml again.
    for f in .joy/items/CT-*.yaml; do
        HEAD8=$(head -c 8 "$f")
        [ "$HEAD8" != "JOYCRYPT" ]
    done
}

@test "joy crypt read decrypts a file to stdout (ADR-040)" {
    joy init --name "Crypt Test" --acronym CT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    mkdir -p secret
    echo "the secret payload" > secret/notes.txt
    joy crypt add "secret/" --passphrase "$TEST_PASSPHRASE" >/dev/null

    # File on disk is now encrypted.
    HEAD8=$(head -c 8 secret/notes.txt)
    [ "$HEAD8" = "JOYCRYPT" ]

    # joy crypt read decrypts to stdout, plaintext never on disk.
    run joy crypt read secret/notes.txt --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"the secret payload"* ]]

    # File is still encrypted on disk after read.
    HEAD8=$(head -c 8 secret/notes.txt)
    [ "$HEAD8" = "JOYCRYPT" ]
}

@test "joy crypt write encrypts stdin into a marked file" {
    joy init --name "Crypt Test" --acronym CT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    mkdir -p secret
    echo "initial" > secret/data.txt
    joy crypt add "secret/" --passphrase "$TEST_PASSPHRASE" >/dev/null

    # Pipe a new value through joy crypt write.
    echo "updated payload" | joy crypt write secret/data.txt --passphrase "$TEST_PASSPHRASE"

    # On disk: encrypted blob.
    HEAD8=$(head -c 8 secret/data.txt)
    [ "$HEAD8" = "JOYCRYPT" ]

    # Roundtrip via read.
    run joy crypt read secret/data.txt --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"updated payload"* ]]
}

@test "joy crypt unlock + lock roundtrip preserves content" {
    joy init --name "Crypt Test" --acronym CT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    mkdir -p secret
    echo "binary file content" > secret/blob.bin
    joy crypt add "secret/" --passphrase "$TEST_PASSPHRASE" >/dev/null

    # Unlock: file becomes plaintext on disk.
    run joy crypt unlock secret/blob.bin --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    HEAD8=$(head -c 8 secret/blob.bin)
    [ "$HEAD8" != "JOYCRYPT" ]
    grep -q "binary file content" secret/blob.bin

    # Lock: file goes back to ciphertext.
    run joy crypt lock secret/blob.bin --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    HEAD8=$(head -c 8 secret/blob.bin)
    [ "$HEAD8" = "JOYCRYPT" ]

    # Decrypt back via read - content matches.
    run joy crypt read secret/blob.bin --passphrase "$TEST_PASSPHRASE"
    [[ "$output" == *"binary file content"* ]]
}

@test "joy crypt edit drives \$EDITOR on a temp file" {
    joy init --name "Crypt Test" --acronym CT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    mkdir -p secret
    echo "before" > secret/notes.txt
    joy crypt add "secret/" --passphrase "$TEST_PASSPHRASE" >/dev/null

    # Use a fake "editor" that appends a line. EDITOR receives the
    # temp path as its single argument.
    EDITOR='sh -c "echo after >> $1" --' \
        joy crypt edit secret/notes.txt --passphrase "$TEST_PASSPHRASE"

    # File on disk is still encrypted; decrypt and check content.
    HEAD8=$(head -c 8 secret/notes.txt)
    [ "$HEAD8" = "JOYCRYPT" ]
    run joy crypt read secret/notes.txt --passphrase "$TEST_PASSPHRASE"
    [[ "$output" == *"before"* ]]
    [[ "$output" == *"after"* ]]
}

@test "joy crypt read on encrypted item without passphrase fails with no-access (ADR-040)" {
    joy init --name "Crypt Test" --acronym CT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    mkdir -p secret
    echo "x" > secret/file.txt
    joy crypt add "secret/" --passphrase "$TEST_PASSPHRASE" >/dev/null

    # Without passphrase, reading must fail at the auth step (rather
    # than producing a crypto-internal error).
    run joy crypt read secret/file.txt --passphrase "wrong wrong wrong wrong wrong wrong"
    [ "$status" -ne 0 ]
}

@test "joy show on encrypted item works with passphrase (ADR-040)" {
    joy init --name "Crypt Test" --acronym CT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy add task "Sensitive item" >/dev/null
    ITEM_FILE=$(ls .joy/items/CT-*.yaml | head -1)
    ID=$(basename "$ITEM_FILE" | sed -E 's/^(CT-[0-9A-Fa-f]+(-[0-9A-Fa-f]+)?)-.*/\1/')

    joy crypt add "$ID" --passphrase "$TEST_PASSPHRASE" >/dev/null

    # On disk: ciphertext.
    HEAD8=$(head -c 8 "$ITEM_FILE")
    [ "$HEAD8" = "JOYCRYPT" ]

    # joy show with passphrase decrypts and renders.
    run joy show "$ID" --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Sensitive item"* ]]
}

@test "joy auth re-locks files left unlocked (ADR-040)" {
    joy init --name "Crypt Test" --acronym CT
    joy auth init --passphrase "$TEST_PASSPHRASE"
    mkdir -p secret
    echo "important" > secret/leak.txt
    joy crypt add "secret/" --passphrase "$TEST_PASSPHRASE" >/dev/null

    # Unlock the file - now plaintext on disk.
    joy crypt unlock secret/leak.txt --passphrase "$TEST_PASSPHRASE" >/dev/null
    HEAD8=$(head -c 8 secret/leak.txt)
    [ "$HEAD8" != "JOYCRYPT" ]

    # Forget about it; deauth and re-auth (typical day-2 flow).
    joy deauth
    run joy auth --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Re-locked"* ]]

    # File is back to ciphertext.
    HEAD8=$(head -c 8 secret/leak.txt)
    [ "$HEAD8" = "JOYCRYPT" ]
}

@test "joy crypt revoke removes a member's wrap" {
    joy init --name "Crypt Test" --acronym CT
    joy auth init --passphrase "$TEST_PASSPHRASE"

    # Self-grant via the auto-create path: `joy crypt add` writes the
    # acting member's own crypt_wraps entry.
    mkdir -p secret
    echo "x" > secret/file.txt
    joy crypt add "secret/" --passphrase "$TEST_PASSPHRASE" >/dev/null
    grep -q "crypt_wraps:" .joy/project.yaml

    run joy crypt revoke test@example.com
    [ "$status" -eq 0 ]
    [[ "$output" == *"Revoked"* ]]
    ! grep -A 3 "test@example.com" .joy/project.yaml | grep -q "crypt_wraps:"
}

# ============================================================
# AI Tool --crypt token flow (JOY-015B-53, JOY-015E-4C)
# ============================================================

@test "joy auth token add --crypt embeds delegation private key" {
    joy init --name "AI Crypt Test" --acronym AC
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:claude@joy --passphrase "$TEST_PASSPHRASE"
    # Auth-only token: no privkey embedded.
    PLAIN=$(joy auth token add ai:claude@joy --passphrase "$TEST_PASSPHRASE" \
        | grep -o 'joy_t_[A-Za-z0-9+/=]*' | head -1)
    [ -n "$PLAIN" ]
    PLAIN_DECODED=$(echo "$PLAIN" | sed 's/^joy_t_//' | base64 -d 2>/dev/null || true)
    [[ "$PLAIN_DECODED" != *"delegation_private_key"* ]]

    # --crypt token: privkey embedded.
    CRYPT=$(joy auth token add ai:claude@joy --crypt --passphrase "$TEST_PASSPHRASE" \
        | grep -o 'joy_t_[A-Za-z0-9+/=]*' | head -1)
    [ -n "$CRYPT" ]
    CRYPT_DECODED=$(echo "$CRYPT" | sed 's/^joy_t_//' | base64 -d 2>/dev/null || true)
    [[ "$CRYPT_DECODED" == *"delegation_private_key"* ]]
    [[ "$CRYPT_DECODED" == *"\"crypt\""* ]]
}

@test "auth-only token redemption produces a 44-byte JOY_SESSION payload" {
    joy init --name "AI Crypt Test" --acronym AC
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:claude@joy --passphrase "$TEST_PASSPHRASE"
    TOKEN=$(joy auth token add ai:claude@joy --passphrase "$TEST_PASSPHRASE" \
        | grep -o 'joy_t_[A-Za-z0-9+/=]*' | head -1)
    OUTPUT=$(joy auth --token "$TOKEN")
    # Extract just the base64 payload from the export line.
    PAYLOAD=$(echo "$OUTPUT" | grep -o 'joy_s_[A-Za-z0-9+/=]*' | head -1 | sed 's/^joy_s_//')
    DECODED_LEN=$(echo "$PAYLOAD" | base64 -d 2>/dev/null | wc -c | tr -d '[:space:]')
    [ "$DECODED_LEN" = "44" ]
}

@test "--crypt token redemption produces a 76-byte JOY_SESSION payload" {
    joy init --name "AI Crypt Test" --acronym AC
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:claude@joy --passphrase "$TEST_PASSPHRASE"
    TOKEN=$(joy auth token add ai:claude@joy --crypt --passphrase "$TEST_PASSPHRASE" \
        | grep -o 'joy_t_[A-Za-z0-9+/=]*' | head -1)
    OUTPUT=$(joy auth --token "$TOKEN")
    PAYLOAD=$(echo "$OUTPUT" | grep -o 'joy_s_[A-Za-z0-9+/=]*' | head -1 | sed 's/^joy_s_//')
    DECODED_LEN=$(echo "$PAYLOAD" | base64 -d 2>/dev/null | wc -c | tr -d '[:space:]')
    [ "$DECODED_LEN" = "76" ]
}

@test "joy crypt grant for AI writes zone-major delegation wraps" {
    joy init --name "AI Crypt Test" --acronym AC
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:claude@joy --passphrase "$TEST_PASSPHRASE"
    # Bootstrap operator's delegation entry by issuing one (auth-only) token.
    joy auth token add ai:claude@joy --passphrase "$TEST_PASSPHRASE" >/dev/null

    # Seed default zone so wraps can be derived.
    mkdir -p secret
    echo "x" > secret/file.txt
    joy crypt add "secret/" --passphrase "$TEST_PASSPHRASE" >/dev/null

    run joy crypt grant ai:claude@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Granted ai:claude@joy"* ]]

    # The wrap landed under crypt.zones.default.delegations.ai:claude@joy.<operator>
    grep -q "delegations:" .joy/project.yaml
    grep -A 6 "delegations:" .joy/project.yaml | grep -q "ai:claude@joy:"
    grep -A 6 "ai:claude@joy:" .joy/project.yaml | grep -q "test@example.com:"
}

@test "joy crypt revoke for AI removes the entire delegations.<ai> map" {
    joy init --name "AI Crypt Test" --acronym AC
    joy auth init --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:claude@joy --passphrase "$TEST_PASSPHRASE"
    joy auth token add ai:claude@joy --passphrase "$TEST_PASSPHRASE" >/dev/null
    mkdir -p secret
    echo "x" > secret/file.txt
    joy crypt add "secret/" --passphrase "$TEST_PASSPHRASE" >/dev/null
    joy crypt grant ai:claude@joy --passphrase "$TEST_PASSPHRASE" >/dev/null

    run joy crypt revoke ai:claude@joy
    [ "$status" -eq 0 ]
    [[ "$output" == *"Revoked"* ]]

    ! grep -A 6 "default:" .joy/project.yaml | grep -q "ai:claude@joy:"
}

# Two more behaviours are deliberately exercised at the Rust unit-test
# level rather than via bats because a portable shell reproduction would
# be brittle:
#   - session.expires = min(session_ttl, token.expires) is verified by
#     ai_session_clamped_to_token_expiry / _uses_session_ttl_when_token
#     _lives_longer in joy-core/auth/session.rs.
#   - The AI-side decrypt round-trip (JOY_SESSION carries the delegation
#     private key, ensure_zone_keys uses it to unwrap a granted zone) is
#     covered through the parse_session_env_full unit test plus the live
#     unwrap_for_member path; bats-driving JOY_SESSION across the
#     run/eval boundary from a captured token string was inconsistent
#     across bash versions.
