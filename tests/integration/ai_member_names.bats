#!/usr/bin/env bats
# An AI member is known by its name: `claude`, not `ai:claude@joy`
# (JI-019D-46). A command typed the old way keeps working and says once
# what the member is called now.

load setup

@test "an AI member is registered under its name, in a file of its own" {
    setup_human_auth
    run joy project member add helper --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Added member helper"* ]]
    [ -n "$(member_file helper)" ]
    ! members_grep -q "ai:helper@joy"
}

@test "the old spelling is taken for the name and says so once" {
    setup_human_auth
    run --separate-stderr joy project member add ai:helper@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Added member helper"* ]]
    [ "$stderr" = "note: ai:helper@joy is now called helper" ]
    [ -n "$(member_file helper)" ]

    # every command that takes a member takes the old spelling
    run --separate-stderr joy auth token add ai:helper@joy --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$stderr" == *"note: ai:helper@joy is now called helper"* ]]
    run joy project member show ai:helper@joy
    [ "$status" -eq 0 ]
    [[ "$output" == *"helper"* ]]
}

@test "output for a program carries no note" {
    setup_human_auth
    joy project member add helper --passphrase "$TEST_PASSPHRASE"
    run --separate-stderr joy project member show ai:helper@joy --json
    [ "$status" -eq 0 ]
    [ -z "$stderr" ]
}

@test "an old id inside a text stays as the person wrote it" {
    setup_human_auth
    joy add task "Named in a comment"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Named in a comment" | awk '{print $1}')
    run --separate-stderr joy comment "$ITEM_ID" "ask ai:helper@joy about this"
    [ "$status" -eq 0 ]
    [ -z "$stderr" ]
    grep -q "ask ai:helper@joy about this" .joy/items/"$ITEM_ID"-*.yaml
}

@test "a token issued under the old spelling still redeems" {
    setup_human_auth
    joy project member add helper --passphrase "$TEST_PASSPHRASE"
    TOKEN=$(joy auth token add ai:helper@joy --passphrase "$TEST_PASSPHRASE" 2>/dev/null | tr -d '"')
    run joy auth --token "$TOKEN" --json
    [ "$status" -eq 0 ]
    [[ "$output" == *'"member":"helper"'* ]]
}
