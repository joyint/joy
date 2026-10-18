#!/usr/bin/env bats
# What an AI member may do is signed by the person who set it up
# (JI-019D-46): joy ai init writes that signature for every member it
# registers, the same one joy project member add writes. Without it an
# AI member's capabilities would be whatever its file says.

load setup

# Fake claude+copilot on PATH so the AI-tool detector finds something
# during these tests.
setup_fake_ai_tools() {
    BIN_DIR="$TEST_DIR/fake-bin"
    mkdir -p "$BIN_DIR"
    for cmd in claude copilot; do
        printf '#!/bin/sh\nexit 0\n' > "$BIN_DIR/$cmd"
        chmod +x "$BIN_DIR/$cmd"
    done
    PATH="$BIN_DIR:$PATH"
}

@test "joy ai init signs what a new AI member may do" {
    setup_human_auth
    setup_fake_ai_tools

    joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null 2>/dev/null

    # The member's file carries the signature of the person acting, and
    # the level that was signed with the capabilities.
    local file
    file="$(member_file claude)"
    grep -q "^granted:" "$file"
    grep -q "  by: test@example.com" "$file"
    grep -q "^level: " "$file"
    # An AI member is brought in by a delegation, not by an invitation.
    ! grep -q "^origin:" "$file"
}

@test "AI member attestation has no enrollment_verifier (no OTP)" {
    setup_human_auth
    setup_fake_ai_tools

    joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null 2>/dev/null

    # AI members authenticate via delegation tokens, not OTP redemption.
    # Their entry should not carry an enrollment_verifier.
    ! members_grep -q "enrollment_verifier:"
}

@test "joy ai init fails fast when no passphrase available and a member needs attestation" {
    setup_human_auth
    setup_fake_ai_tools

    # The session `auth init` made would carry the seed (the auth gate,
    # JOY-02B2-65), so it is ended first. No session, no --passphrase,
    # no terminal -> the gate has nothing to unlock with and must refuse
    # rather than silently write an unattested member.
    joy deauth
    run joy ai init </dev/null 2>&1
    [ "$status" -ne 0 ]
    [[ "$output" == *"run \`joy auth\`"* ]]
    ! members_grep -q "claude"
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
