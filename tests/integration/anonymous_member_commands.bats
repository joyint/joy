#!/usr/bin/env bats
# JOY-01C8-2D / ADR-042: member-touching auth commands must work in anonymous
# mode, not just open mode. Before the type-bound member-access refactor, the
# member map was indexed directly by the cleartext e-mail at many call sites.
# In anonymous mode the map is keyed by an opaque id, so those lookups failed:
# `joy auth token add` and `joy project member add --with-token` (both share
# create_delegation_token) reported the operator as "not a registered project
# member" or "guard denied", even though the member table listed them. An AI
# could be registered but never delegated a token -- anonymous mode was broken
# end to end past `ai init`. These tests pin the token paths in anonymous mode
# and assert no operator e-mail ever reaches the committed project.yaml.

load setup

TEST_EMAIL="test@example.com"

_anon() {
    JOY_PASSPHRASE="$TEST_PASSPHRASE" joy init --anonymous --name "Secret" >/dev/null
}

@test "auth token add issues a token in an anonymous project" {
    _anon
    joy project member add ai:claude@joy --passphrase "$TEST_PASSPHRASE" >/dev/null

    run joy auth token add ai:claude@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"joy_t_"* ]]
    [[ "$output" != *"not a registered project member"* ]]
    [[ "$output" != *"guard denied"* ]]
    # The operator's e-mail must never appear anywhere under .joy/.
    run grep -rl "$TEST_EMAIL" .joy/
    [ "$status" -ne 0 ]
}

@test "member add --with-token registers an AI and issues a token in anonymous mode" {
    _anon

    run joy project member add ai:copilot@joy --with-token --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"joy_t_"* ]]

    run env JOY_PASSPHRASE="$TEST_PASSPHRASE" joy project member
    [[ "$output" == *"ai:copilot@joy"* ]]

    run grep -rl "$TEST_EMAIL" .joy/
    [ "$status" -ne 0 ]
}

@test "an AI token issued in anonymous mode redeems and the AI can act" {
    _anon
    joy project member add ai:claude@joy --passphrase "$TEST_PASSPHRASE" >/dev/null
    token=$(joy auth token add ai:claude@joy --passphrase "$TEST_PASSPHRASE" | tr -d '"')

    # Redeem: `joy auth --token` prints an `export JOY_SESSION=...` line.
    eval "$(joy auth --token "$token")"
    [ -n "$JOY_SESSION" ]

    # The AI now acts with its session: create an item.
    run joy add task "work from anonymous AI" --session "$JOY_SESSION"
    [ "$status" -eq 0 ]

    # The operator's e-mail must not appear anywhere under .joy/, including the
    # `delegated-by:` of the actor recorded in the item and the log. At rest the
    # delegating operator is stored by opaque member id; MemberRef resolves it
    # back to the e-mail only for an authorized viewer (asserted below).
    run grep -rl "$TEST_EMAIL" .joy/
    [ "$status" -ne 0 ]

    # The at-rest actor carries the opaque id, not the e-mail.
    run grep -rh "delegated-by:m-" .joy/items
    [ "$status" -eq 0 ]

    # ... and an authorized viewer (members.yaml unlocked) sees the operator
    # e-mail resolved back, never the raw opaque id.
    id=$(joy ls --json 2>/dev/null | grep -oiE '[A-Z]+-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{2}' | head -1)
    run env JOY_PASSPHRASE="$TEST_PASSPHRASE" joy show "$id"
    [ "$status" -eq 0 ]
    [[ "$output" == *"delegated-by:$TEST_EMAIL"* ]]
    [[ "$output" != *"delegated-by:m-"* ]]
}

@test "member add refuses a human member in an anonymous project, without leaking PII" {
    _anon

    # Anonymous human onboarding is not yet supported (JOY-01C3-A7): a naive
    # e-mail-keyed insert would write cleartext PII into the committed
    # project.yaml, so register_member refuses it instead.
    run joy project member add newperson@example.com --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    ! grep -q "newperson@example.com" .joy/project.yaml
}
