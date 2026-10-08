#!/usr/bin/env bats
#
# Integration tests for member attestation (JOY-00FA-A5 epic).
# Written upfront as the TDD contract for the 8 child items.
# All tests are expected to FAIL until the feature lands.
#
# Scenarios mapped to the 8-point design:
#   1. joy init creates founder without attestation
#   2. joy project member add creates OTP + attestation signed by founder
#   3. joy auth --otp sets passphrase and silently reverse-attests founder
#   4. Further joy project member add works without further reverse-attestation
#   5. joy project member rm <self> blocked with manage-list error
#   6. Manage removing another member inherits the removed member's attestations
#   7. Manually injected member without attestation fails joy auth
#   8. Tampered attestation signature fails joy auth

load setup

FOUNDER_PASSPHRASE="correct horse battery staple extra words"
ALICE_PASSPHRASE="alpha bravo charlie delta echo foxtrot"
BOB_PASSPHRASE="golf hotel india juliett kilo lima"
CAROL_PASSPHRASE="mike november oscar papa quebec romeo"
EVE_PASSPHRASE="echo foxtrot golf hotel india juliett"

# Extract the OTP code from 'joy project member add' output.
# Expected format in output: a line like "One-time password: XXX-XXX-XXX".
extract_otp() {
    echo "$1" | sed -n 's/^[[:space:]]*One-time password:[[:space:]]*\([A-Za-z0-9-]*\).*$/\1/p' | head -1
}

# Establish the founder identity using the git email from setup (test@example.com).
setup_founder() {
    joy init --name "Attestation Test" --acronym AT
    joy auth init --passphrase "$FOUNDER_PASSPHRASE"
}

# Add a new member and capture the emitted OTP in MEMBER_OTP.
# Args: $1 = member email, $2 = capabilities (optional, default: all)
add_member_capture_otp() {
    local email="$1"
    local caps="${2:-}"
    local out
    if [ -n "$caps" ]; then
        out=$(joy project member add "$email" --capabilities "$caps" --passphrase "$FOUNDER_PASSPHRASE")
    else
        out=$(joy project member add "$email" --passphrase "$FOUNDER_PASSPHRASE")
    fi
    MEMBER_OTP=$(extract_otp "$out")
}

# Act as `email` from here on: name this repository's git config as
# them AND authenticate as them, opening their session.
#
# Since the operator's 2026-09-19 correction (JOY-02AE-1A)
# `resolve_identity` reads `git config user.email` again, and the
# device pin it used to read instead is gone: a bare `joy` command after
# this acts as `email` because THIS repository's git config names them.
become_member() {
    local email="$1"
    local passphrase="$2"
    git config user.email "$email"
    joy auth --user "$email" --passphrase "$passphrase"
}

# An invited member's first act in their own checkout: redeem the
# invitation, which sets their passphrase and enrols them, then name
# them in this repository's git config so the bare commands that follow
# act as them (JOY-02AE-1A). The OTP proves the
# invitation; `--user` is the address the invitee types.
enroll_member() {
    local email="$1"
    local passphrase="$2"
    local otp="${3:-$MEMBER_OTP}"
    joy auth --otp "$otp" --user "$email" --passphrase "$passphrase"
    git config user.email "$email"
}

# ============================================================
# 1. joy init creates founder without attestation
# ============================================================

@test "the founder has no origin after joy init + auth init" {
    setup_founder
    # The founder is the one person nobody invited.
    run members_grep -c "^origin:"
    [ "$output" = "0" ]
    # Founder has verify_key and kdf_nonce from auth init.
    members_grep -q "verify_key:"
    members_grep -q "kdf_nonce:"
}

# ============================================================
# 2. joy project member add creates OTP + attestation signed by founder
# ============================================================

@test "member add emits OTP and writes an origin signed by the founder" {
    setup_founder
    run joy project member add alice@example.com --passphrase "$FOUNDER_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"One-time password:"* ]]
    local otp
    otp=$(extract_otp "$output")
    [ -n "$otp" ]

    # Alice's file says who invited her: the founder, with a signature.
    local file
    file="$(member_file alice@example.com)"
    grep -q "^origin:" "$file"
    grep -q "  attester: test@example.com" "$file"
    grep -q "  signature: " "$file"
    # enrollment_verifier is recorded (alice still has one, pre-redemption).
    members_grep -q "enrollment_verifier:"
    # Only the founder has a verify_key at this point; alice has none.
    [ "$(members_grep -c '^verify_key:')" = "1" ]
}

@test "member add without manage-member passphrase fails" {
    setup_founder
    run joy project member add alice@example.com --passphrase "wrong wrong wrong wrong wrong wrong"
    [ "$status" -ne 0 ]
    [[ "$output" == *"passphrase"* ]]
}

# ============================================================
# 3. joy auth --otp sets passphrase and silently reverse-attests founder
# ============================================================

@test "otp redemption sets the passphrase and leaves the invitation as it was signed" {
    setup_founder
    add_member_capture_otp alice@example.com
    [ -n "$MEMBER_OTP" ]

    local signed
    signed=$(grep "  signature: " "$(member_file alice@example.com)")

    # Alice redeems OTP and sets her passphrase.
    run joy auth --otp "$MEMBER_OTP" --user alice@example.com --passphrase "$ALICE_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" != *"founder"* ]]

    # Alice's own copy of the invitation is cleared; her origin keeps the
    # hash it was signed over.
    run members_grep -E "^enrollment_verifier:"
    [ "$status" -ne 0 ]
    # Both alice and founder now have verify_keys at member-level.
    [ "$(members_grep -cE '^verify_key:')" = "2" ]

    # The origin is untouched by the redemption, and the founder is
    # still the one person without one.
    [ "$(grep "  signature: " "$(member_file alice@example.com)")" = "$signed" ]
    ! grep -q "^origin:" "$(member_file test@example.com)"
}

@test "otp redemption finds its member behind a forge alias address" {
    # JOY-0257-FC / JP-00BF-94: the redeemer's clone carries GitHub's
    # privacy alias, not the invited address. The OTP is the identity
    # proof during enrolment, so the INVITED slot enrolls anyway and no
    # member appears under the alias.
    setup_founder
    add_member_capture_otp alice@example.com
    [ -n "$MEMBER_OTP" ]

    run joy auth --otp "$MEMBER_OTP" --user "12345+alice@users.noreply.github.com" \
        --passphrase "$ALICE_PASSPHRASE"
    [ "$status" -eq 0 ]

    # invitation spent on the invited slot, both members enrolled
    run members_grep -E "^enrollment_verifier:"
    [ "$status" -ne 0 ]
    [ "$(members_grep -cE '^verify_key:')" = "2" ]
    # no second identity under the alias
    ! members_grep -q "users.noreply.github.com"
}

# ============================================================
# 4. Further member adds work without further reverse-attestation
# ============================================================

@test "subsequent adds do not re-attest the founder" {
    setup_founder
    # Alice is added, redeems, reverse-attests founder.
    add_member_capture_otp alice@example.com
    enroll_member alice@example.com "$ALICE_PASSPHRASE"

    # Capture founder's current attestation signature.
    local before_sig
    before_sig=$(members_grep -A10 "test@example.com:" | grep "signature:" | head -1)

    # Alice (now manage) adds bob.
    become_member test@example.com "$FOUNDER_PASSPHRASE"   # go back to manage
    add_member_capture_otp bob@example.com
    enroll_member bob@example.com "$BOB_PASSPHRASE"

    # Founder's attestation is unchanged.
    local after_sig
    after_sig=$(members_grep -A10 "test@example.com:" | grep "signature:" | head -1)
    [ "$before_sig" = "$after_sig" ]
}

# ============================================================
# 5. Self-remove blocked with manage-list error
# ============================================================

@test "self-remove blocked and error lists manage members" {
    setup_founder
    # Alice must have `manage` to trigger the self-remove guard at all;
    # the member-add default excludes manage/delete, so grant explicitly.
    add_member_capture_otp alice@example.com all
    enroll_member alice@example.com "$ALICE_PASSPHRASE"

    # Alice attempts to remove herself.
    run joy project member rm alice@example.com --passphrase "$ALICE_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"Cannot remove yourself"* ]] || [[ "$output" == *"another manage"* ]]
    # Error lists other manage members by email.
    [[ "$output" == *"test@example.com"* ]]
}

# ============================================================
# 6. Manage remove inherits attestations of the removed member
# ============================================================

@test "manage remove inherits attested members" {
    setup_founder
    # Alice and Bob both need `manage`: alice to add carol, bob to remove
    # alice. Carol stays on defaults but the grep uses -A20 so it still
    # finds her attester line past her capability block.
    add_member_capture_otp alice@example.com all
    enroll_member alice@example.com "$ALICE_PASSPHRASE"

    # Alice (as manage) adds carol; alice is carol's attester.
    MEMBER_OTP=$(joy project member add carol@example.com --passphrase "$ALICE_PASSPHRASE" \
        | sed -n 's/^[[:space:]]*One-time password:[[:space:]]*\([A-Za-z0-9-]*\).*$/\1/p' | head -1)
    enroll_member carol@example.com "$CAROL_PASSPHRASE"
    grep -q "  attester: alice@example.com" "$(member_file carol@example.com)"

    # Founder adds bob as another manage member.
    become_member test@example.com "$FOUNDER_PASSPHRASE"
    MEMBER_OTP=$(joy project member add bob@example.com --capabilities all --passphrase "$FOUNDER_PASSPHRASE" \
        | sed -n 's/^[[:space:]]*One-time password:[[:space:]]*\([A-Za-z0-9-]*\).*$/\1/p' | head -1)
    enroll_member bob@example.com "$BOB_PASSPHRASE"

    # Bob removes alice. Alice attested carol, so carol must be
    # re-attested by bob as part of the removal.
    run joy project member rm alice@example.com --passphrase "$BOB_PASSPHRASE"
    [ "$status" -eq 0 ]

    # Alice's file is gone; bob signed for carol in her place.
    [ -z "$(member_file alice@example.com)" ]
    grep -q "  attester: bob@example.com" "$(member_file carol@example.com)"
}

# ============================================================
# 7. Manually injected member without attestation fails joy auth
# ============================================================

@test "manually injected member fails joy auth with clear error" {
    setup_founder
    # Add a legitimate second member so the attestation-required invariant
    # (fires once two verify_keys exist) applies to every member.
    add_member_capture_otp alice@example.com
    enroll_member alice@example.com "$ALICE_PASSPHRASE"

    # Simulate an edit by hand: a member file for eve and her id in the
    # list, without going through 'joy project member add'.
    printf 'email: eve@attacker.com\ncapabilities: all\nupdated: 2026-01-01T00:00:00Z\n' \
        > .joy/members/m-eveeveevee.yaml
    awk '/^members:/ { print; print "- m-eveeveevee"; next } { print }' \
        .joy/project.yaml > .joy/project.yaml.tmp \
        && mv .joy/project.yaml.tmp .joy/project.yaml

    # Eve names herself (the project now lists her) and tries to
    # bootstrap her auth: joy auth init sets her verify_key, and then she
    # authenticates. joy auth rejects her because nobody invited her and
    # she is not the founder.
    joy auth init --user eve@attacker.com --passphrase "$EVE_PASSPHRASE"
    run joy auth --user eve@attacker.com --passphrase "$EVE_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"was not invited by anyone"* ]]
    # Error points the user at the recovery path.
    [[ "$output" == *"re-add"* ]]
}

# ============================================================
# 8. Tampered attestation signature fails joy auth
# ============================================================

@test "a tampered origin signature fails joy auth" {
    setup_founder
    add_member_capture_otp alice@example.com
    enroll_member alice@example.com "$ALICE_PASSPHRASE"

    # Flip one hex character in the signature of alice's origin (perl
    # for a portable in-place edit).
    perl -i -pe 'BEGIN {$done=0} if (!$done && /^\s+signature:\s*([0-9a-f])/) { $c = $1 eq "0" ? "1" : "0"; s/signature:\s*[0-9a-f]/signature: $c/; $done=1 }' "$(member_file alice@example.com)"

    # Alice tries to auth with her valid passphrase; the origin does not
    # verify.
    run joy auth --passphrase "$ALICE_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"does not verify"* ]]
}

# ============================================================
# 5. An invitation must be proven, not sidestepped
# ============================================================

@test "an invited member cannot skip the OTP via joy auth init" {
    setup_founder
    add_member_capture_otp alice@example.com
    [ -n "$MEMBER_OTP" ]

    # Setting up a fresh identity instead of redeeming would leave alice with a
    # self-chosen verify_key. The attestation signs e-mail, capabilities and the
    # enrollment verifier but NOT the key, so nothing downstream would notice.
    run joy auth init --user alice@example.com --passphrase "$ALICE_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"one-time password is required"* ]]
    # Only the founder is enrolled; alice's slot is untouched.
    [ "$(members_grep -cE '^verify_key:')" = "1" ]

    # Redeeming the invitation is the way in.
    run joy auth --otp "$MEMBER_OTP" --user alice@example.com --passphrase "$ALICE_PASSPHRASE"
    [ "$status" -eq 0 ]
    [ "$(members_grep -cE '^verify_key:')" = "2" ]
}
