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
