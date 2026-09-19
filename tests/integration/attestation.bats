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
# Since the operator's 2026-09-19 correction (JOY-02AE-1A, correcting
# D3.9) `resolve_identity` reads `git config user.email` again, and the
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
# act as them (JOY-02AE-1A, correcting D3.9). The OTP proves the
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

@test "founder entry has no attestation after joy init + auth init" {
    setup_founder
    # Founder is the sole member, trust root, no attestation expected.
    run grep -c "attestation:" .joy/project.yaml
    [ "$output" = "0" ]
    # Founder has verify_key and kdf_nonce from auth init.
    grep -q "verify_key:" .joy/project.yaml
    grep -q "kdf_nonce:" .joy/project.yaml
}

# ============================================================
# 2. joy project member add creates OTP + attestation signed by founder
# ============================================================

@test "member add emits OTP and writes attestation signed by founder" {
    setup_founder
    run joy project member add alice@example.com --passphrase "$FOUNDER_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"One-time password:"* ]]
    local otp
    otp=$(extract_otp "$output")
    [ -n "$otp" ]

    # project.yaml now contains an attestation block naming test@example.com
    # (the founder) as attester.
    grep -q "attestation:" .joy/project.yaml
    grep -q "attester: test@example.com" .joy/project.yaml
    # enrollment_verifier is recorded (alice still has one, pre-redemption).
    grep -q "enrollment_verifier:" .joy/project.yaml
    # Only the founder has a verify_key at this point; alice has none.
    [ "$(grep -c '^    verify_key:' .joy/project.yaml)" = "1" ]
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

@test "otp redemption sets passphrase and reverse-attests founder silently" {
    setup_founder
    add_member_capture_otp alice@example.com
    [ -n "$MEMBER_OTP" ]

    # Founder currently has no attestation.
    run bash -c 'grep -B1 "test@example.com:" .joy/project.yaml | head -3'

    # Alice redeems OTP and sets her passphrase.
    run joy auth --otp "$MEMBER_OTP" --user alice@example.com --passphrase "$ALICE_PASSPHRASE"
    [ "$status" -eq 0 ]
    # Redemption output should be minimal - no explicit mention of
    # reverse-attesting the founder (silent behavior per the 8-point design).
    [[ "$output" != *"reverse-attesting"* ]]
    [[ "$output" != *"founder"* ]]

    # Alice's member-level enrollment_verifier is cleared (attestation signed_fields under the pinned otp_hash key
    # may still reference it as historical record - that's 8-space indent
    # and not matched by the member-level regex).
    run grep -E "^    enrollment_verifier:" .joy/project.yaml
    [ "$status" -ne 0 ]
    # Both alice and founder now have verify_keys at member-level.
    [ "$(grep -cE '^    verify_key:' .joy/project.yaml)" = "2" ]

    # Founder now carries an attestation naming alice as attester.
    grep -q "attester: alice@example.com" .joy/project.yaml
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
    run grep -E "^    enrollment_verifier:" .joy/project.yaml
    [ "$status" -ne 0 ]
    [ "$(grep -cE '^    verify_key:' .joy/project.yaml)" = "2" ]
    # no second identity under the alias
    ! grep -q "users.noreply.github.com" .joy/project.yaml
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
    before_sig=$(grep -A10 "test@example.com:" .joy/project.yaml | grep "signature:" | head -1)

    # Alice (now manage) adds bob.
    become_member test@example.com "$FOUNDER_PASSPHRASE"   # go back to manage
    add_member_capture_otp bob@example.com
    enroll_member bob@example.com "$BOB_PASSPHRASE"

    # Founder's attestation is unchanged.
    local after_sig
    after_sig=$(grep -A10 "test@example.com:" .joy/project.yaml | grep "signature:" | head -1)
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
    grep -A20 "carol@example.com:" .joy/project.yaml | grep -q "attester: alice@example.com"

    # Founder adds bob as another manage member.
    become_member test@example.com "$FOUNDER_PASSPHRASE"
    MEMBER_OTP=$(joy project member add bob@example.com --capabilities all --passphrase "$FOUNDER_PASSPHRASE" \
        | sed -n 's/^[[:space:]]*One-time password:[[:space:]]*\([A-Za-z0-9-]*\).*$/\1/p' | head -1)
    enroll_member bob@example.com "$BOB_PASSPHRASE"

    # Bob removes alice. Alice attested carol, so carol must be
    # re-attested by bob as part of the removal.
    run joy project member rm alice@example.com --passphrase "$BOB_PASSPHRASE"
    [ "$status" -eq 0 ]

    # Alice's entry is gone; carol's attester is now bob.
    ! grep -q "^  alice@example.com:" .joy/project.yaml
    grep -A20 "carol@example.com:" .joy/project.yaml | grep -q "attester: bob@example.com"
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

    # Simulate a manual yaml edit: insert eve as a member without
    # attestation, without going through 'joy project member add'.
    # Inserted just before the top-level 'created:' line so yaml stays valid.
    # awk for BSD/GNU portability: insert eve before the top-level created: line.
    awk '/^created:/ && !done {
        print "  eve@attacker.com:";
        print "    capabilities:";
        print "      manage: {}";
        done = 1
    } { print }' .joy/project.yaml > .joy/project.yaml.tmp \
        && mv .joy/project.yaml.tmp .joy/project.yaml

    # Eve names herself (the member map now lists her) and tries to
    # bootstrap her auth: joy auth init sets her verify_key, and then she
    # authenticates. The attestation check at joy auth rejects her
    # because her entry has no attestation.
    joy auth init --user eve@attacker.com --passphrase "$EVE_PASSPHRASE"
    run joy auth --user eve@attacker.com --passphrase "$EVE_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"attestation"* ]] || [[ "$output" == *"tampered"* ]] || [[ "$output" == *"not valid"* ]]
    # Error points the user at the recovery path.
    [[ "$output" == *"manage member"* ]] || [[ "$output" == *"re-add"* ]]
}

# ============================================================
# 8. Tampered attestation signature fails joy auth
# ============================================================

@test "silent auto-seal attests existing members on first post-upgrade auth" {
    # Simulate a pre-feature project: members with verify_keys but no
    # attestations anywhere. Dev joins via direct yaml edit + auth init
    # before we introduced attestations (we achieve the same state by
    # stripping attestations from a fresh project).
    setup_founder
    add_member_capture_otp alice@example.com
    enroll_member alice@example.com "$ALICE_PASSPHRASE"

    # Back to the founder BEFORE the project is put into its pre-feature
    # state, so the authentication below is the first one the auto-seal
    # can happen in.
    become_member test@example.com "$FOUNDER_PASSPHRASE"

    # Strip every attestation block (simulating pre-feature state).
    python3 -c "
import re, sys
with open('.joy/project.yaml') as f:
    text = f.read()
text = re.sub(r'    attestation:\n(      .*\n)*', '', text)
with open('.joy/project.yaml', 'w') as f:
    f.write(text)
"
    ! grep -q "attestation:" .joy/project.yaml

    # Deauth and re-auth as founder. Auto-seal triggers: founder signs
    # attestations for everyone else (just alice here).
    joy deauth
    run joy auth --passphrase "$FOUNDER_PASSPHRASE"
    [ "$status" -eq 0 ]

    # Alice now has an attestation naming test@example.com as attester;
    # founder remains unattested (trust root of the sealed state).
    # -A30 reaches past alice's capability list (default excludes
    # manage/delete, so the capability block spans several lines).
    grep -A30 "alice@example.com:" .joy/project.yaml | grep -q "attester: test@example.com"
    # Verify silent: auth output should not mention sealing.
    [[ "$output" != *"seal"* ]]
    [[ "$output" != *"migration"* ]]
}

@test "tampered attestation signature fails joy auth" {
    setup_founder
    add_member_capture_otp alice@example.com
    enroll_member alice@example.com "$ALICE_PASSPHRASE"

    # Flip one hex character in alice's attestation signature.
    # Uses perl for cross-platform sed-style in-place edit on the first
    # hex digit of the 'signature:' line in the attestation block.
    perl -i -pe 'BEGIN {$done=0} if (!$done && /^\s+signature:\s*([0-9a-f])/) { $c = $1 eq "0" ? "1" : "0"; s/signature:\s*[0-9a-f]/signature: $c/; $done=1 }' .joy/project.yaml

    # Alice tries to auth with valid passphrase; attestation check fails.
    run joy auth --passphrase "$ALICE_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"attestation"* ]] || [[ "$output" == *"tampered"* ]] || [[ "$output" == *"not valid"* ]]
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
    [ "$(grep -cE '^    verify_key:' .joy/project.yaml)" = "1" ]

    # Redeeming the invitation is the way in.
    run joy auth --otp "$MEMBER_OTP" --user alice@example.com --passphrase "$ALICE_PASSPHRASE"
    [ "$status" -eq 0 ]
    [ "$(grep -cE '^    verify_key:' .joy/project.yaml)" = "2" ]
}
