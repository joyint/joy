#!/usr/bin/env bats
# JOY-01BC-A9 / ADR-042: in anonymous mode joy release resolves opaque
# contributor ids to the e-mail in the LOCAL terminal view for an authorized
# viewer, but keeps the opaque id in published / persisted artifacts (the
# --markdown notes meant for a forge release, and the saved .yaml). Publishing
# must never undo the anonymization.

load setup

TEST_EMAIL="test@example.com"

# Anonymous project with one closed item and a recorded release v0.1.0.
#
# Every joy call detaches stdin from any TTY (`</dev/null`). A human session is
# TTY-bound (ADR-023): if the founder session were created on the suite's
# terminal, the piped `joy release record` below (stdin is a pipe, so no TTY)
# would fail the session's TTY check and be denied. Detaching stdin everywhere
# keeps the whole flow on one context (no TTY), so it passes both headless (CI)
# and when the suite is run attached to a real terminal.
_anon_release() {
    JOY_PASSPHRASE="$TEST_PASSPHRASE" joy init --anonymous --name "Secret" </dev/null >/dev/null
    local id
    id=$(joy add task "ship it" </dev/null | grep -oiE '[A-Z]+-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{2}' | head -1)
    joy start "$id" </dev/null >/dev/null
    joy submit "$id" </dev/null >/dev/null
    joy approve "$id" </dev/null >/dev/null 2>&1 || true
    joy close "$id" </dev/null >/dev/null 2>&1 || true
    printf 'y\n' | joy release record v0.1.0 >/dev/null
}

@test "joy release show resolves the contributor to the e-mail for an authorized viewer" {
    _anon_release
    run env JOY_PASSPHRASE="$TEST_PASSPHRASE" joy release show v0.1.0
    [ "$status" -eq 0 ]
    [[ "$output" == *"$TEST_EMAIL"* ]]
    [[ "$output" != *"m-"* ]]
}

@test "joy release show when locked shows neither the e-mail nor a raw id" {
    _anon_release
    # Drop the session so the members.yaml zone key is no longer available.
    rm -rf "$XDG_STATE_HOME"
    run joy release show v0.1.0
    [ "$status" -eq 0 ]
    # Fail-safe: a locked viewer sees an auth request, never the opaque id.
    [[ "$output" != *"$TEST_EMAIL"* ]]
    [[ "$output" != *"m-"* ]]
}

@test "joy release show --markdown stays anonymous even with the passphrase" {
    _anon_release
    run env JOY_PASSPHRASE="$TEST_PASSPHRASE" joy release show v0.1.0 --markdown
    [ "$status" -eq 0 ]
    [[ "$output" != *"$TEST_EMAIL"* ]]
    [[ "$output" == *"m-"* ]]
}

@test "the saved release .yaml carries the opaque id, never the e-mail" {
    _anon_release
    run grep -rl "$TEST_EMAIL" .joy/releases/
    [ "$status" -ne 0 ]
    run grep -hE 'id: m-[a-z2-7]{10}' .joy/releases/SEC-v0.1.0.yaml
    [ "$status" -eq 0 ]
}
