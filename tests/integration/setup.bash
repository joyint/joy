#!/usr/bin/env bash
# Common setup for bats integration tests.
# Sources: https://bats-core.readthedocs.io/

# Require bats >= 1.5.0 so `run` accepts result-asserting flags
# (e.g. `run -0`, `run --separate-stderr`) without a BW02 warning.
# Loaded by every test file via `load setup`, so this applies project-wide.
bats_require_minimum_version 1.5.0

# Ensure joy binary is available (prefer debug build for speed)
JOY_BIN="${JOY_BIN:-$(pwd)/target/debug/joy}"
if [ ! -x "$JOY_BIN" ]; then
    JOY_BIN="$(command -v joy)"
fi
export PATH="$(dirname "$JOY_BIN"):$PATH"

TEST_PASSPHRASE="correct horse battery staple extra words"

# The address every project here is founded by. `setup` writes it into
# the sandbox git config, which is where `joy init` OFFERS it (D3.9 keeps
# git config as a prefill and nothing more), and `act_as_founder` names
# it when a test has to come back from acting as somebody else.
FOUNDER_EMAIL="test@example.com"

# Create a temporary project directory for each test
setup() {
    TEST_DIR="$(mktemp -d)"
    cd "$TEST_DIR" || exit 1
    git init --quiet
    git config user.email "$FOUNDER_EMAIL"
    git config user.name "Test User"
    # Isolate per-user state between tests. axoupdater reads its
    # install receipt from ~/.config/<pkg>/ (hard-coded relative to
    # $HOME on Unix, ignoring XDG_CONFIG_HOME), so a real receipt left
    # behind by a prior curl|sh install would otherwise leak into the
    # test run. Overriding HOME isolates that too.
    export HOME="$TEST_DIR"
    export XDG_STATE_HOME="$TEST_DIR/.state"
    export XDG_CONFIG_HOME="$TEST_DIR/.config"
    # Overriding HOME hides ~/.gitconfig but not /etc/gitconfig, and a CI
    # image that puts a git identity there makes the cases about a
    # MISSING identity pass on a laptop and fail on the runner. Ignoring
    # the system file completes the isolation the lines above intend.
    export GIT_CONFIG_NOSYSTEM=1
}

# Clean up after each test
teardown() {
    cd /
    rm -rf "$TEST_DIR"
    unset HOME
    unset XDG_STATE_HOME
    unset XDG_CONFIG_HOME
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
    joy project member add "$ai_member" --passphrase "$TEST_PASSPHRASE" 2>/dev/null || true
    # joy auth token add wraps the token in double quotes; strip them
    # for code paths that pass AI_TOKEN to other commands as a bare
    # value. `joy auth --token` itself accepts both forms.
    AI_TOKEN=$(joy auth token add "$ai_member" --passphrase "$TEST_PASSPHRASE" | tr -d '"')
    # Auth as AI; eval sets JOY_SESSION
    eval $(joy auth --token "$AI_TOKEN")
    SAVED_JOY_SESSION="$JOY_SESSION"
}

DEV_PASSPHRASE="alpha bravo charlie delta echo foxtrot"

# The one-time password from `joy project member add` output.
extract_otp() {
    grep -oE '[A-Z0-9]{4}-[A-Z0-9]{4}-[A-Z0-9]{4}' | head -1
}

# Act as `member` from here on: authenticate as them, which opens their
# session AND pins them as the member this device acts as (D3.9).
#
# This is how a test says who is working now. It used to be `git config
# user.email <member>`, and since package J11 that changes nothing at
# all: no joy command decides an identity from git config any more, so a
# test that switched that way went on acting as whoever was
# authenticated last. Naming the member is the only way left, and it is
# also the way a person does it.
act_as() {
    local member="$1"
    local passphrase="$2"
    joy auth --user "$member" --passphrase "$passphrase"
}

# Back to the member the project was founded by.
act_as_founder() {
    act_as "$FOUNDER_EMAIL" "$TEST_PASSPHRASE"
}

# A fresh clone, or a second machine: the project travels in the
# repository, this device's own state does not. Both the sessions and the
# member pin live in it, so afterwards nothing on this machine says who
# acts here.
forget_this_device() {
    rm -rf "$XDG_STATE_HOME/joy"
}

# Enrol another member (e.g. dev@example.com) by redeeming their invitation,
# the real flow: a manage member adds them (emitting an OTP), the invitee
# proves it. Setting a fresh identity WITHOUT the OTP is refused (an invited
# slot cannot be claimed with a self-chosen key). Pass the OTP captured at
# `member add`, or set DEV_OTP by convention. Switches back afterwards.
setup_member_auth() {
    local member="$1"
    local passphrase="$2"
    local otp="${3:-$DEV_OTP}"
    # The invitee names themselves, the way an invited person does on
    # their own machine. The redemption pins them here, so the way back
    # to the founder names the founder.
    joy auth --otp "$otp" --user "$member" --passphrase "$passphrase"
    act_as_founder
}

# Switch back to human identity.
switch_to_human() {
    unset JOY_SESSION
}

# Switch to AI identity (requires JOY_SESSION set by setup_ai_session).
switch_to_ai() {
    export JOY_SESSION="$SAVED_JOY_SESSION"
}

# Portable in-place sed. Args: <expr> <file...>. Works on BSD (macOS)
# and GNU because both accept `-i.bak`; the resulting `.bak` files are
# cleaned up afterwards.
sed_inplace() {
    local expr="$1"
    shift
    sed -i.bak "$expr" "$@"
    local f
    for f in "$@"; do
        rm -f "${f}.bak"
    done
}

# Run a shell command inside a real PTY, portably across BSD (macOS)
# and util-linux `script` flavours.
pty_run() {
    local cmd="$1"
    if script --version 2>&1 | grep -q util-linux; then
        script -qc "$cmd" /dev/null
    else
        script -q /dev/null sh -c "$cmd"
    fi
}
