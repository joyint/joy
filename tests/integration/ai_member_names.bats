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

# ---------------------------------------------------------------
# joy ai add: one AI member, ready to work, with one passphrase
# ---------------------------------------------------------------

@test "joy ai add registers the member, sets its tool up and prints its token" {
    setup_human_auth
    run joy ai add claude --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Added member claude"* ]]
    [[ "$output" == *"joy_t_"* ]]
    [[ "$output" == *"Claude Code is set up in this checkout."* ]]
    grep -q "^adapter: claude" "$(member_file claude)"
    [ -f .claude/CLAUDE.md ]
    # the tool starts in the mode the member's level means
    grep -q '"defaultMode": "bypassPermissions"' .claude/settings.json

    # the printed token redeems, and the AI acts
    TOKEN=$(printf '%s\n' "$output" | grep -o 'joy_t_[A-Za-z0-9+/=]*' | head -1)
    eval "$(joy auth --token "$TOKEN")"
    run joy add task "Written by the new member"
    [ "$status" -eq 0 ]
}

@test "joy ai add gives a member of its own name the tool and model it is told" {
    setup_human_auth
    run joy ai add reviewer --adapter claude --model opus --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    grep -q "^adapter: claude" "$(member_file reviewer)"
    grep -q "^model: opus" "$(member_file reviewer)"
}

@test "joy ai add asks which tool runs a member that is not named after one" {
    setup_human_auth
    run joy ai add reviewer --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"say which tool runs it with --adapter"* ]]
    [ -z "$(member_file reviewer)" ]
}

@test "a level lowered for the project reaches the tool's own settings at once" {
    setup_human_auth
    joy ai add claude --passphrase "$TEST_PASSPHRASE" >/dev/null
    joy project member edit claude --project --level proposing --passphrase "$TEST_PASSPHRASE"
    grep -q '"defaultMode": "plan"' .claude/settings.json
}
