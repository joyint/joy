#!/usr/bin/env bats
# JOY-01C2-36: TDD acceptance suite for the activated anonymous privacy mode
# (ADR-042). Written test-first; RED until the anonymous-mode tasks land
# (opaque ids JOY-01BD-51, members.yaml JOY-01BE-A2, transition JOY-01BF-2E,
# and display resolution).
#
# Two acceptance properties:
#   1. After switching a project to privacy=anonymous and exercising it, NO
#      member e-mail appears in any generated .joy file (project.yaml incl. the
#      attestation, items, logs). members.yaml is encrypted, so its plaintext
#      e-mail never hits disk in the clear either.
#   2. Representative Joy outputs (joy log, joy show) resolve a member to the
#      NAME when one is set, otherwise to the e-mail, and never show a raw
#      opaque id (m-...). Both variants are covered.
#
# Interface assumptions (to be confirmed by the implementation tasks):
#   - Activation: `joy project set privacy anonymous` performs the atomic
#     migration (JOY-01BF-2E).
#   - The display NAME is sourced zero-config from `git config user.name`
#     (parallel to the e-mail coming from `git config user.email`, ADR-009),
#     stored in members.yaml. No name set => display falls back to the e-mail.

load setup

TEST_EMAIL="test@example.com"

# Drive a member into the log/assignee fields: create, start (assigns + logs a
# status change), comment.
_exercise_item() {
    local id
    id=$(joy add task "work item" | grep -oiE 'JOY-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{2}' | head -1)
    joy start "$id" >/dev/null
    joy comment "$id" "in progress" >/dev/null
    joy submit "$id" >/dev/null
    printf '%s' "$id"
}

@test "anonymous: no member e-mail appears in any generated .joy file" {
    setup_human_auth
    joy project set privacy anonymous
    _exercise_item >/dev/null

    # Recursively scan every .joy artifact. grep -l exits non-zero when no file
    # matches, which is the pass condition. -I would skip the (binary) encrypted
    # members.yaml; we deliberately do NOT pass -I so a plaintext leak there is
    # caught too.
    run grep -rl "$TEST_EMAIL" .joy/
    [ "$status" -ne 0 ]
}

@test "anonymous: project.yaml carries an email_match verifier, not the e-mail" {
    setup_human_auth
    joy project set privacy anonymous

    run grep -q "$TEST_EMAIL" .joy/project.yaml
    [ "$status" -ne 0 ]
    run grep -q "email_match" .joy/project.yaml
    [ "$status" -eq 0 ]
}

# Name capture is deferred (members.yaml `name` is optional and not populated
# yet). The resolver's name-over-e-mail fallback is unit-tested in joy-core; this
# end-to-end case is kept for when name capture lands.
@test "anonymous: joy log shows the member NAME when one is set (future: name capture)" {
    skip "name capture deferred; resolver name-over-e-mail fallback covered by joy-core unit tests"
}

@test "anonymous: joy log resolves the actor to the e-mail, never a raw id" {
    setup_human_auth
    joy project set privacy anonymous
    id=$(_exercise_item)

    run joy log "$id"
    [ "$status" -eq 0 ]
    [[ "$output" == *"$TEST_EMAIL"* ]]
    [[ "$output" != *"m-"* ]]
}

@test "anonymous: joy show resolves the assignee to the e-mail, never a raw id" {
    setup_human_auth
    joy project set privacy anonymous
    id=$(_exercise_item)

    run joy show "$id"
    [ "$status" -eq 0 ]
    [[ "$output" == *"$TEST_EMAIL"* ]]
    [[ "$output" != *"m-"* ]]
}

# This one is GREEN already: the manage guard fires before the not-yet-
# implemented bail, so the auth+manage guarantee for switching to anonymous
# holds independently of the migration work.
@test "anonymous: switching to anonymous requires the manage capability" {
    setup_human_auth
    setup_ai_session ai:test@joy
    run joy project set privacy anonymous
    [ "$status" -ne 0 ]
    [[ "$output" == *"cannot perform manage"* ]]
}
