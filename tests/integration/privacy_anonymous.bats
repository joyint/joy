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
    local title="${1:-work item}"
    local id
    id=$(joy add task "$title" | grep -oiE '[A-Z]+-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{2}' | head -1)
    joy start "$id" >/dev/null
    joy comment "$id" "in progress" >/dev/null
    joy submit "$id" >/dev/null
    printf '%s' "$id"
}

# Switch the project to anonymous and re-authenticate as the now-anon member
# (the prior session was keyed by the e-mail, which is no longer the map key).
_go_anonymous() {
    JOY_PASSPHRASE="$TEST_PASSPHRASE" joy project set privacy anonymous >/dev/null
    joy auth --passphrase "$TEST_PASSPHRASE" >/dev/null
}

# Switch back to open and re-authenticate under the restored e-mail key.
_go_open() {
    JOY_PASSPHRASE="$TEST_PASSPHRASE" joy project set privacy open >/dev/null
    joy auth --passphrase "$TEST_PASSPHRASE" >/dev/null
}

# Pass condition: at least one .joy file still carries the e-mail.
_emails_present() { grep -rlq "$TEST_EMAIL" .joy/; }

@test "anonymous: no member e-mail appears in any generated .joy file" {
    setup_human_auth
    _go_anonymous
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
    _go_anonymous

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
    _go_anonymous
    id=$(_exercise_item)

    # The viewer resolves ids to e-mails only with their own key; a read command
    # never prompts, so the passphrase is supplied non-interactively.
    run env JOY_PASSPHRASE="$TEST_PASSPHRASE" joy log --item "$id"
    [ "$status" -eq 0 ]
    [[ "$output" == *"$TEST_EMAIL"* ]]
    [[ "$output" != *"m-"* ]]
}

@test "anonymous: joy show resolves the assignee to the e-mail, never a raw id" {
    setup_human_auth
    _go_anonymous
    id=$(_exercise_item)

    run env JOY_PASSPHRASE="$TEST_PASSPHRASE" joy show "$id"
    [ "$status" -eq 0 ]
    [[ "$output" == *"$TEST_EMAIL"* ]]
    [[ "$output" != *"m-"* ]]
}

# This one is GREEN already: the manage guard fires before the not-yet-
# implemented bail, so the auth+manage guarantee for switching to anonymous
# holds independently of the migration work.
@test "journey: e-mails present in open, gone in anonymous, restored, gone again" {
    setup_human_auth
    _exercise_item >/dev/null

    # Open mode: the e-mail is present on disk.
    run _emails_present
    [ "$status" -eq 0 ]

    # Anonymous: every e-mail is scrubbed and an opaque id takes its place.
    _go_anonymous
    run _emails_present
    [ "$status" -ne 0 ]
    run grep -rlE 'm-[a-z2-7]{10}' .joy/items .joy/logs
    [ "$status" -eq 0 ]

    # A fresh action while anonymous still leaks no e-mail.
    _exercise_item "second item" >/dev/null
    run _emails_present
    [ "$status" -ne 0 ]

    # Back to open: every e-mail (including the one minted while anonymous) is restored.
    _go_open
    run _emails_present
    [ "$status" -eq 0 ]

    # And switching to anonymous a second time scrubs them once more.
    _go_anonymous
    run _emails_present
    [ "$status" -ne 0 ]
}

@test "anonymous: a multi-member project still authenticates after the switch (frozen attestation)" {
    setup_human_auth
    # A second member with their own identity; both end up attested in open mode.
    DEV_OTP=$(joy project member add dev@example.com --passphrase "$TEST_PASSPHRASE" | extract_otp)
    setup_member_auth dev@example.com "$DEV_PASSPHRASE"

    JOY_PASSPHRASE="$TEST_PASSPHRASE" joy project set privacy anonymous >/dev/null

    # Founder re-authenticates: the attestation signature must verify over the
    # real e-mail (email_for), even though project.yaml now stores an id.
    run joy auth --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Authenticated as test@example.com"* ]]

    # The co-member authenticates too.
    git config user.email "dev@example.com"
    run joy auth --passphrase "$DEV_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Authenticated as dev@example.com"* ]]
    git config user.email "test@example.com"

    # No member e-mail leaked into project.yaml.
    run grep -cE 'test@example|dev@example' .joy/project.yaml
    [ "$output" = "0" ]
}

@test "anonymous: adding an AI member after the switch keeps project.yaml e-mail-free" {
    # Regression: registering a member in an anonymous project must resolve the
    # founder via the privacy-aware member key, not a direct members.get(email)
    # lookup. In anonymous mode the map is keyed by an opaque id, so the direct
    # lookup (1) failed to find the founder when deriving the keypair to sign the
    # attestation, and (2) recorded the founder's cleartext e-mail as the
    # attester, leaking it back into the committed project.yaml. Unlike the
    # multi-member case above (member added in open mode, then frozen by the
    # switch), this adds the member while the project is already anonymous, which
    # is the path that exercises both fixes.
    setup_human_auth
    _go_anonymous

    run joy project member add ai:copilot@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" != *"not a registered project member"* ]]

    # The AI member is registered ...
    run env JOY_PASSPHRASE="$TEST_PASSPHRASE" joy project member
    [[ "$output" == *"ai:copilot@joy"* ]]

    # ... and the founder's e-mail never appears anywhere under .joy/ -- in
    # particular not as the attester inside project.yaml's attestation block.
    run grep -rl "$TEST_EMAIL" .joy/
    [ "$status" -ne 0 ]
}

@test "anonymous: erase severs id->e-mail resolution but keeps the audit trail (GDPR Art. 17)" {
    setup_human_auth
    DEV_OTP=$(joy project member add dev@example.com --passphrase "$TEST_PASSPHRASE" | extract_otp)
    setup_member_auth dev@example.com "$DEV_PASSPHRASE"
    JOY_PASSPHRASE="$TEST_PASSPHRASE" joy project set privacy anonymous >/dev/null
    joy auth --passphrase "$TEST_PASSPHRASE" >/dev/null

    # Before erasure the co-member's e-mail resolves for the authenticated viewer.
    run joy project
    [[ "$output" == *"dev@example.com"* ]]

    JOY_PASSPHRASE="$TEST_PASSPHRASE" joy project member erase dev@example.com

    # After erasure it resolves nowhere anymore.
    run joy project
    [[ "$output" != *"dev@example.com"* ]]
    # But both opaque member entries remain in project.yaml (audit trail intact).
    run grep -cE '^  m-[a-z2-7]{10}:' .joy/project.yaml
    [ "$output" -ge 2 ]
}

@test "anonymous: switching to anonymous requires the manage capability" {
    setup_human_auth
    setup_ai_session ai:test@joy
    run joy project set privacy anonymous
    [ "$status" -ne 0 ]
    [[ "$output" == *"cannot perform manage"* ]]
}
