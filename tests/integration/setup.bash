#!/usr/bin/env bash
# Common setup for bats integration tests.
# Sources: https://bats-core.readthedocs.io/

# Ensure joy binary is available (prefer debug build for speed)
JOY_BIN="${JOY_BIN:-$(pwd)/target/debug/joy}"
if [ ! -x "$JOY_BIN" ]; then
    JOY_BIN="$(command -v joy)"
fi
export PATH="$(dirname "$JOY_BIN"):$PATH"

TEST_PASSPHRASE="correct horse battery staple extra words"

# Create a temporary project directory for each test
setup() {
    TEST_DIR="$(mktemp -d)"
    cd "$TEST_DIR" || exit 1
    git init --quiet
    git config user.email "test@example.com"
    git config user.name "Test User"
    # Isolate sessions between tests
    export XDG_STATE_HOME="$TEST_DIR/.state"
}

# Clean up after each test
teardown() {
    cd /
    rm -rf "$TEST_DIR"
    unset XDG_STATE_HOME
}

# Setup human auth and return to authenticated state.
setup_human_auth() {
    joy init --name "Test Project" 2>/dev/null
    joy auth init --passphrase "$TEST_PASSPHRASE"
}

# Setup AI member, create token, authenticate AI.
# After this, joy commands run as the AI member.
# Sets JOY_SESSION (SSH-agent pattern) and saves AI_TOKEN for re-auth.
setup_ai_session() {
    local ai_member="${1:-ai:test@joy}"
    # Add member if not already registered (idempotent)
    joy project member add "$ai_member" 2>/dev/null || true
    AI_TOKEN=$(joy auth token add "$ai_member" --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')
    # Auth as AI -- eval sets JOY_SESSION
    eval $(joy auth --token "$AI_TOKEN")
    SAVED_JOY_SESSION="$JOY_SESSION"
}

DEV_PASSPHRASE="alpha bravo charlie delta echo foxtrot"

# Authenticate another member (e.g. dev@example.com).
# Switches git email, runs auth init, switches back.
setup_member_auth() {
    local member="$1"
    local passphrase="$2"
    local original_email
    original_email=$(git config user.email)
    git config user.email "$member"
    joy auth init --passphrase "$passphrase"
    git config user.email "$original_email"
    # Re-authenticate as original (dev's auth init overwrote the session file)
    joy auth --passphrase "$TEST_PASSPHRASE"
}

# Switch back to human identity.
switch_to_human() {
    unset JOY_SESSION
}

# Switch to AI identity (requires JOY_SESSION set by setup_ai_session).
switch_to_ai() {
    export JOY_SESSION="$SAVED_JOY_SESSION"
}
