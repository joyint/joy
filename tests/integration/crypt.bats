#!/usr/bin/env bats
# Integration tests for the human-member Crypt user journey:
# init -> auth -> add -> encrypt -> ls/show -> edit -> comment ->
# status -> grant a second member -> revoke -> locked-row rendering.
#
# Covers JOY-0176-DA: every read/write command must unlock zone keys
# via load_context(); a regression there used to break joy edit /
# comment / start / close on any item in a Crypt zone.

load setup

PASS_BOB="alpha bravo charlie delta echo"
PASS_ALICE="november oscar papa quebec romeo"

setup_bob_with_crypt() {
    git config user.email "bob@example.com"
    joy init --acronym CT --name "Crypt Journey Test" >/dev/null
    joy auth init --passphrase "$PASS_BOB" >/dev/null
}

@test "crypt journey: edit on an encrypted item with JOY_PASSPHRASE" {
    setup_bob_with_crypt
    joy add task "secret task" --effort 2 >/dev/null
    joy crypt add CT-0001 --passphrase "$PASS_BOB" >/dev/null

    JOY_PASSPHRASE="$PASS_BOB" run joy edit CT-0001 --description "updated body"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Updated"* ]]

    JOY_PASSPHRASE="$PASS_BOB" run joy show CT-0001
    [ "$status" -eq 0 ]
    [[ "$output" == *"updated body"* ]]
}

@test "crypt journey: comment + status + close on encrypted item" {
    setup_bob_with_crypt
    joy add task "another secret" >/dev/null
    joy crypt add CT-0001 --passphrase "$PASS_BOB" >/dev/null

    JOY_PASSPHRASE="$PASS_BOB" run joy comment CT-0001 "secret note"
    [ "$status" -eq 0 ]

    JOY_PASSPHRASE="$PASS_BOB" run joy start CT-0001
    [ "$status" -eq 0 ]
    [[ "$output" == *"in-progress"* ]]

    JOY_PASSPHRASE="$PASS_BOB" run joy close CT-0001
    [ "$status" -eq 0 ]
    [[ "$output" == *"closed"* ]]

    JOY_PASSPHRASE="$PASS_BOB" run joy show CT-0001
    [ "$status" -eq 0 ]
    [[ "$output" == *"secret note"* ]]
    # Status renders as the short form "don" in the show table.
    [[ "$output" == *"don"* ]]
}

@test "crypt journey: log strips item content but keeps structural events" {
    setup_bob_with_crypt
    joy add task "content that must not leak" >/dev/null
    joy crypt add CT-0001 --passphrase "$PASS_BOB" >/dev/null
    JOY_PASSPHRASE="$PASS_BOB" joy comment CT-0001 "private note that must not leak" >/dev/null
    JOY_PASSPHRASE="$PASS_BOB" joy start CT-0001 >/dev/null

    # No user-authored payload anywhere in the log file.
    run grep -F "content that must not leak" .joy/logs/*.log
    [ "$status" -ne 0 ]
    run grep -F "private note that must not leak" .joy/logs/*.log
    [ "$status" -ne 0 ]

    # Structural events still recorded.
    run grep -E "CT-0001.* item.created" .joy/logs/*.log
    [ "$status" -eq 0 ]
    run grep -E "CT-0001.* comment.added" .joy/logs/*.log
    [ "$status" -eq 0 ]
    run grep -F "new -> in-progress" .joy/logs/*.log
    [ "$status" -eq 0 ]
}

@test "crypt journey: second member sees locked rows until granted" {
    setup_bob_with_crypt
    joy add task "shared secret" >/dev/null
    joy crypt add CT-0001 --passphrase "$PASS_BOB" >/dev/null

    # Bob (still the active git identity) registers Alice as a member,
    # capturing her invitation OTP; Alice then redeems it herself.
    ALICE_OTP=$(joy project member add alice@example.com --passphrase "$PASS_BOB" | extract_otp)
    git config user.email "alice@example.com"
    joy auth --otp "$ALICE_OTP" --passphrase "$PASS_ALICE" >/dev/null

    # Alice has no zone access yet. ls must list the locked row, not error.
    run joy ls
    [ "$status" -eq 0 ]
    [[ "$output" == *"CT-0001"* ]]
    [[ "$output" == *"encrypted, no access"* ]]
    [[ "$output" == *"default"* ]]

    # Alice cannot read the body.
    run joy show CT-0001
    [ "$status" -ne 0 ]

    # Bob grants Alice. Switch git identity back to Bob to act as him.
    git config user.email "bob@example.com"
    joy auth --passphrase "$PASS_BOB" >/dev/null
    joy crypt grant alice@example.com --passphrase "$PASS_BOB" >/dev/null

    # Switch to Alice and re-auth (auth file is per-process).
    git config user.email "alice@example.com"
    joy auth --passphrase "$PASS_ALICE" >/dev/null

    # Alice now reads the item with her own passphrase.
    JOY_PASSPHRASE="$PASS_ALICE" run joy show CT-0001
    [ "$status" -eq 0 ]
    [[ "$output" == *"shared secret"* ]]

    # Alice can also edit the encrypted item.
    JOY_PASSPHRASE="$PASS_ALICE" run joy edit CT-0001 --description "alice was here"
    [ "$status" -eq 0 ]
    JOY_PASSPHRASE="$PASS_ALICE" run joy show CT-0001
    [[ "$output" == *"alice was here"* ]]
}

@test "crypt journey: plaintext-only project never prompts" {
    git config user.email "bob@example.com"
    joy init --acronym CT --name "Plain Project" >/dev/null
    joy auth init --passphrase "$PASS_BOB" >/dev/null
    joy add task "plain item" >/dev/null

    # No JOY_PASSPHRASE, no --passphrase, no terminal: must still
    # succeed because the pre-check (JOY-0173-B3) skips the prompt.
    run joy edit CT-0001 --description "plain update"
    [ "$status" -eq 0 ]
    run joy comment CT-0001 "plain note"
    [ "$status" -eq 0 ]
    run joy show CT-0001
    [ "$status" -eq 0 ]
    [[ "$output" == *"plain update"* ]]
    [[ "$output" == *"plain note"* ]]
}
