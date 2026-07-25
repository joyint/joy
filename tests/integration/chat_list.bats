#!/usr/bin/env bats
# joy chat list/show reads the git-native chats on refs/joy/chats
# (JOY-01F3, ADR JAPP-00DC-FC).

load setup

@test "joy chat list and show render a chat" {
    joy init --name "Chat Test" 2>/dev/null
    run joy chat list
    [ "$status" -eq 0 ]
    [[ "$output" == *"No chats."* ]]

    # chats live on refs/joy/chats now, not a working-tree file: create one
    # through the CLI (which writes the ref) and read it back
    run joy chat send general "moin"
    [ "$status" -eq 0 ]
    run joy chat send general "hi Horst"
    [ "$status" -eq 0 ]

    run joy chat list
    [ "$status" -eq 0 ]
    [[ "$output" == *"general"* ]]
    [[ "$output" == *"General"* ]]

    run joy chat show general
    [ "$status" -eq 0 ]
    [[ "$output" == *"moin"* ]]
    [[ "$output" == *"hi Horst"* ]]
}

@test "joy chat send/leave/delete lifecycle from the terminal" {
    joy init --name "Chat Life" 2>/dev/null
    run joy chat send general "moin team"
    [ "$status" -eq 0 ]
    run joy chat show general
    [[ "$output" == *"moin team"* ]]

    # general cannot be left, but IS deletable since 2026-07 (operator):
    # for-all freezes it AND marks the deleter; as the only human that
    # completes the set, the file is collected, and a FRESH empty General
    # takes its place (ensure_general)
    run joy chat leave general
    [ "$status" -ne 0 ]
    run joy chat delete general --for-all
    [ "$status" -eq 0 ]
    run joy chat show general
    [[ "$output" != *"moin team"* ]]
}

@test "chat verbs sync refs/joy/chats over a shared origin (JOY-0227-5E)" {
    # two clones of one bare origin: A sends, B reads it without any manual
    # ref plumbing, B replies, A sees both (round-trip, ADR JAPP-00DC-FC)
    ORIGIN="$BATS_TEST_TMPDIR/origin.git"
    A="$BATS_TEST_TMPDIR/a"
    B="$BATS_TEST_TMPDIR/b"
    git init -q --bare "$ORIGIN"

    mkdir -p "$A" && cd "$A"
    git init -q .
    git config user.email "a@example.com"
    git config user.name "A"
    git remote add origin "$ORIGIN"
    run joy init --name "Sync Test"
    [ "$status" -eq 0 ]
    git add -A && git commit -qm "seed [no-item]"
    git push -q origin HEAD

    run joy chat send general "moin from A"
    [ "$status" -eq 0 ]
    # the send pushed the chats ref itself
    git -C "$ORIGIN" show-ref | grep -q "refs/joy/chats"

    git clone -q "$ORIGIN" "$B"
    cd "$B"
    git config user.email "b@example.com"
    git config user.name "B"
    # a plain clone carries no custom refs; the read fetches + adopts
    run joy chat show general
    [ "$status" -eq 0 ]
    [[ "$output" == *"moin from A"* ]]

    run joy chat send general "reply from B"
    [ "$status" -eq 0 ]

    cd "$A"
    run joy chat show general
    [ "$status" -eq 0 ]
    [[ "$output" == *"moin from A"* ]]
    [[ "$output" == *"reply from B"* ]]
}

@test "joy chat ls shows LAST@ME/UNREAD and --mine filters to my mentions (JOY-0225/0226)" {
    PP="correct horse battery staple extra words"
    run joy init --name "Mention Test"
    [ "$status" -eq 0 ]
    # my identity for the mention matching; sealed chats need the
    # passphrase on every verb from here on
    joy auth init --passphrase "$PP"

    run joy chat send general "moin ohne mention" --passphrase "$PP"
    [ "$status" -eq 0 ]
    run joy chat send general "hey @test@example.com schau mal" --passphrase "$PP"
    [ "$status" -eq 0 ]

    # ls is the verb now; list stays as a quiet alias
    run joy chat ls --passphrase "$PP"
    [ "$status" -eq 0 ]
    [[ "$output" == *"LAST@ME"* ]]
    [[ "$output" == *"UNREAD"* ]]
    [[ "$output" == *"general"* ]]
    run joy chat list --passphrase "$PP"
    [ "$status" -eq 0 ]
    [[ "$output" == *"general"* ]]

    # --mine: General qualifies (a mention at me exists); a mention-free
    # chat would be filtered out
    run joy chat ls --mine --passphrase "$PP"
    [ "$status" -eq 0 ]
    [[ "$output" == *"general"* ]]

    # reading clears the unread count
    run joy chat read general --passphrase "$PP"
    [ "$status" -eq 0 ]
    run joy chat ls --mine --passphrase "$PP"
    [ "$status" -eq 0 ]
}
