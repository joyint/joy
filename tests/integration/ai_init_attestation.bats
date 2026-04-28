#!/usr/bin/env bats
# Verify that joy ai init produces attested AI-member entries, signed
# by the acting human. Without this, joy ai init bypassed the
# attestation chain that joy project member add enforces, leaving
# AI members unverifiable and opening a yaml-edit attack surface.

load setup

# Fake claude+gh on PATH so the AI-tool detector finds something
# during these tests.
setup_fake_ai_tools() {
    BIN_DIR="$TEST_DIR/fake-bin"
    mkdir -p "$BIN_DIR"
    for cmd in claude gh; do
        printf '#!/bin/sh\nexit 0\n' > "$BIN_DIR/$cmd"
        chmod +x "$BIN_DIR/$cmd"
    done
    PATH="$BIN_DIR:$PATH"
}

@test "joy ai init writes attestation block on new AI members" {
    setup_human_auth
    setup_fake_ai_tools

    joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null 2>/dev/null

    # Attestation block must be present.
    grep -q "attestation:" .joy/project.yaml
    grep -q "attester: test@example.com" .joy/project.yaml
    # And the signed_fields must name the AI member as the attestee.
    grep -q "email: ai:claude@joy" .joy/project.yaml
}

@test "AI member attestation has no enrollment_verifier (no OTP)" {
    setup_human_auth
    setup_fake_ai_tools

    joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null 2>/dev/null

    # AI members authenticate via delegation tokens, not OTP redemption.
    # Their entry should not carry an enrollment_verifier.
    ! grep -q "enrollment_verifier:" .joy/project.yaml
}

@test "joy ai init fails fast when no passphrase available and a member needs attestation" {
    setup_human_auth
    setup_fake_ai_tools

    # No --passphrase, no stdin -> derive_acting_keypair has nothing
    # to read and must surface an error rather than silently writing
    # an unattested member.
    run joy ai init </dev/null 2>&1
    [ "$status" -ne 0 ]
    ! grep -q "ai:claude@joy" .joy/project.yaml
}

@test "joy ai init does not prompt for passphrase when no new members are added" {
    setup_human_auth
    setup_fake_ai_tools

    # First run registers members.
    joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null 2>/dev/null

    # Second run has nothing new to register -> derive_acting_keypair
    # is never called -> --passphrase is unnecessary.
    run joy ai init </dev/null 2>/dev/null
    [ "$status" -eq 0 ]
}
