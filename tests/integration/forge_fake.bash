#!/usr/bin/env bash
# The forge boundary for the integration tests (JOY-0298-E4).
#
# The connectors speak HTTP themselves now (design D2.8), so the marked
# stub is a small server on 127.0.0.1 plus a `forges.yaml` that points
# one host at it. `gh` stays a stub too, but only for what it still is:
# a source of a TOKEN (decision 19), never an HTTP client.

# Start the fake forge. Sets FAKE_FORGE_DIR, FAKE_FORGE_PORT and
# FAKE_FORGE_BASE.
start_fake_forge() {
    FAKE_FORGE_DIR="$TEST_DIR/fake-forge"
    mkdir -p "$FAKE_FORGE_DIR"
    : > "$FAKE_FORGE_DIR/calls"
    : > "$FAKE_FORGE_DIR/release_body"
    # fd 3 is bats' own output stream: a child that keeps it open holds
    # the whole run open after the last test, so it is closed here
    # along with the two the server would write to.
    python3 "$BATS_TEST_DIRNAME/fake_forge.py" "$FAKE_FORGE_DIR" >/dev/null 2>&1 3>&- &
    FAKE_FORGE_PID=$!
    local waited=0
    while [ ! -s "$FAKE_FORGE_DIR/port" ] && [ "$waited" -lt 100 ]; do
        sleep 0.05
        waited=$((waited + 1))
    done
    [ -s "$FAKE_FORGE_DIR/port" ] || { echo "the fake forge did not start" >&2; return 1; }
    FAKE_FORGE_PORT="$(cat "$FAKE_FORGE_DIR/port")"
    FAKE_FORGE_BASE="http://127.0.0.1:$FAKE_FORGE_PORT"
    export FAKE_FORGE_DIR FAKE_FORGE_PORT FAKE_FORGE_BASE
}

stop_fake_forge() {
    [ -n "${FAKE_FORGE_PID:-}" ] && kill "$FAKE_FORGE_PID" 2>/dev/null
    FAKE_FORGE_PID=""
    return 0
}

# Point one host at the fake through forges.yaml (design D2.5). HOME and
# XDG_CONFIG_HOME are the test's own, so this file is the connector's.
point_forge_at_fake() {
    local host="${1:-github.com}" kind="${2:-github}"
    mkdir -p "$XDG_CONFIG_HOME/joy"
    cat > "$XDG_CONFIG_HOME/joy/forges.yaml" <<YAML
- host: $host
  kind: $kind
  api_base: $FAKE_FORGE_BASE
YAML
}

# The gh stub: a TOKEN source and nothing else (decision 19).
install_gh_token_stub() {
    local token="${1:-gho_test-token}"
    STUB_DIR="$TEST_DIR/stub-bin"
    mkdir -p "$STUB_DIR"
    cat > "$STUB_DIR/gh" <<STUB
#!/bin/sh
case "\$1 \$2" in
"auth token") echo "$token" ;;
*) exit 1 ;;
esac
STUB
    chmod +x "$STUB_DIR/gh"
    export PATH="$STUB_DIR:$PATH"
}

# The files that load this helper end every case by stopping the fake
# first, then doing what setup.bash's own teardown does. `load setup`
# runs before `load forge_fake`, so this definition is the one bats
# calls; a server left running would otherwise outlive the whole run.
teardown() {
    stop_fake_forge
    cd /
    rm -rf "$TEST_DIR"
    unset HOME
    unset XDG_STATE_HOME
    unset XDG_CONFIG_HOME
}
