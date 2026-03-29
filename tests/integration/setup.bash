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
# Sets AI_TOKEN for later use.
setup_ai_session() {
    local ai_member="${1:-ai:test@joy}"
    # Add member if not already registered (idempotent)
    joy project member add "$ai_member" 2>/dev/null || true
    AI_TOKEN=$(joy auth create-token "$ai_member" --passphrase "$TEST_PASSPHRASE" \
        | sed -n 's/^  \(joy_t_.*\)/\1/p')
    # Auth as AI (creates session)
    joy auth --token "$AI_TOKEN"
    # Set JOY_TOKEN so resolve_identity picks up the AI session
    export JOY_TOKEN="$AI_TOKEN"
}

# Switch back to human identity.
switch_to_human() {
    unset JOY_TOKEN
}

# Switch to AI identity (requires AI_TOKEN set by setup_ai_session).
switch_to_ai() {
    export JOY_TOKEN="$AI_TOKEN"
}
