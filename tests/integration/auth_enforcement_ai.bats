#!/usr/bin/env bats
# JOY-00BD-9E: focused negative-test suite for AI write enforcement.
# Every way an AI member must be denied a write must be covered here.

load setup

@test "AI write denied without JOY_SESSION env var" {
    setup_human_auth
    joy add task "Target"
    ITEM=$(joy ls 2>/dev/null | grep Target | awk '{print $1}')

    setup_ai_session ai:test@joy
    unset JOY_SESSION

    # The human session may still be valid. The point of this test
    # is the negative one for the AI: without JOY_SESSION the actor
    # is NOT recognised as the AI, so any comment that does land
    # must not be attributed to ai:test@joy.
    joy comment "$ITEM" "should not write" 2>/dev/null || true
    ! grep -q "author: ai:test@joy" .joy/items/${ITEM}-*.yaml
}

@test "AI write denied when JOY_SESSION carries a bogus session id" {
    setup_human_auth
    joy add task "Target"
    ITEM=$(joy ls 2>/dev/null | grep Target | awk '{print $1}')

    setup_ai_session ai:test@joy
    # Replace JOY_SESSION with a syntactically valid but unknown id.
    bogus="joy_s_$(printf 'A%.0s' {1..64})"
    JOY_SESSION="$bogus" joy comment "$ITEM" "should not be ai" 2>/dev/null || true
    ! grep -q "author: ai:test@joy" .joy/items/${ITEM}-*.yaml
}

@test "AI write denied after delegation rotation" {
    setup_human_auth
    joy add task "Target"
    ITEM=$(joy ls 2>/dev/null | grep Target | awk '{print $1}')

    setup_ai_session ai:test@joy
    # Save the AI session so we can re-export it after the human
    # rotates the delegation.
    AI_SESSION="$JOY_SESSION"
    unset JOY_SESSION

    # Rotate the delegation as the human (manage capability).
    joy ai rotate ai:test@joy --passphrase "$TEST_PASSPHRASE" >/dev/null

    # Re-export the OLD AI session: it is bound to the previous
    # delegation key and must no longer authenticate the AI.
    export JOY_SESSION="$AI_SESSION"
    joy comment "$ITEM" "post-rotation write" 2>/dev/null || true
    ! grep -q "author: ai:test@joy" .joy/items/${ITEM}-*.yaml
}

@test "AI write denied when token is for a different project" {
    setup_human_auth
    joy add task "Target"
    ITEM=$(joy ls 2>/dev/null | grep Target | awk '{print $1}')
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE" >/dev/null
    OTHER_TOKEN=$(joy auth token add ai:test@joy --passphrase "$TEST_PASSPHRASE" \
        | grep '^joy_t_')

    # Move to a fresh project and issue a session there.
    SECOND_DIR="$(mktemp -d)"
    cd "$SECOND_DIR"
    git init --quiet
    git config user.email "test@example.com"
    git config user.name "Test User"
    setup_human_auth                       # different project, same OS user
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE" >/dev/null

    # Try to redeem the FIRST project's token in this second project.
    run joy auth --token "$OTHER_TOKEN"
    [ "$status" -ne 0 ]
    cd "$TEST_DIR"
}

@test "AI session is single-shell-bound: sibling shell does not act as AI" {
    setup_human_auth
    joy add task "Target"
    ITEM=$(joy ls 2>/dev/null | grep Target | awk '{print $1}')

    setup_ai_session ai:test@joy
    # Save the env var and clear the local one to simulate a sibling shell.
    SAVED="$JOY_SESSION"
    unset JOY_SESSION

    # Sibling shell: write may succeed as the human, but must not be
    # attributed to the AI.
    joy comment "$ITEM" "sibling shell" 2>/dev/null || true
    ! grep -q "author: ai:test@joy" .joy/items/${ITEM}-*.yaml

    # Restoring JOY_SESSION must let the same AI act again.
    export JOY_SESSION="$SAVED"
    run joy comment "$ITEM" "original shell"
    [ "$status" -eq 0 ]
    grep -q "author: ai:test@joy" .joy/items/${ITEM}-*.yaml
}
