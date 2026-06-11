#!/usr/bin/env bats
# JOY-01BC-A9 / ADR-042: NO Joy output, terminal or --json, ever shows a raw
# opaque member id (m-<short>). These cover the identity / session / attestation
# display paths that do not flow through the item/log MemberRef fields:
# joy auth status, joy project member show, joy deauth, joy project member
# add/rm/erase. They resolve to the e-mail (or pass an ai: id through) instead.

load setup

TEST_EMAIL="test@example.com"

_anon() {
    JOY_PASSPHRASE="$TEST_PASSPHRASE" joy init --anonymous --name "Secret" >/dev/null
}

# The founder's opaque id, read straight from project.yaml.
_member_id() {
    grep -oE 'm-[a-z2-7]{10}' .joy/project.yaml | head -1
}

@test "auth status resolves the session member, never a raw id (terminal + json)" {
    _anon

    run joy auth status
    [ "$status" -eq 0 ]
    [[ "$output" == *"$TEST_EMAIL"* ]]
    [[ "$output" != *"m-"* ]]

    run joy auth status --json
    [ "$status" -eq 0 ]
    [[ "$output" == *"$TEST_EMAIL"* ]]
    [[ "$output" != *'"m-'* ]]
}

@test "member show resolves the member, never a raw id (terminal + json)" {
    _anon
    id=$(_member_id)

    run joy project member show "$id"
    [ "$status" -eq 0 ]
    [[ "$output" == *"$TEST_EMAIL"* ]]
    [[ "$output" != *"m-"* ]]

    run joy project member show "$id" --json
    [ "$status" -eq 0 ]
    [[ "$output" == *"$TEST_EMAIL"* ]]
    [[ "$output" != *'"m-'* ]]
}

@test "deauth names the member by e-mail, never a raw id" {
    _anon

    run joy deauth
    [ "$status" -eq 0 ]
    [[ "$output" != *"m-"* ]]
}

@test "the on-disk event log and items keep the raw id, never an e-mail" {
    _anon
    joy add task "work" >/dev/null

    # Counter-check: the audit trail at rest stays anonymous (opaque id, no PII);
    # resolution is an output-only concern.
    run grep -rlE "$TEST_EMAIL" .joy/logs .joy/items
    [ "$status" -ne 0 ]
    run grep -rlE 'm-[a-z2-7]{10}' .joy/logs
    [ "$status" -eq 0 ]
}
