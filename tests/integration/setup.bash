#!/usr/bin/env bash
# Common setup for bats integration tests.
# Sources: https://bats-core.readthedocs.io/

# Ensure joy binary is available (prefer debug build for speed)
JOY_BIN="${JOY_BIN:-$(pwd)/target/debug/joy}"
if [ ! -x "$JOY_BIN" ]; then
    JOY_BIN="$(command -v joy)"
fi
export PATH="$(dirname "$JOY_BIN"):$PATH"

# Create a temporary project directory for each test
setup() {
    TEST_DIR="$(mktemp -d)"
    cd "$TEST_DIR" || exit 1
    git init --quiet
    git config user.email "test@example.com"
    git config user.name "Test User"
}

# Clean up after each test
teardown() {
    cd /
    rm -rf "$TEST_DIR"
}
