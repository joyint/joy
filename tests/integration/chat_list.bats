#!/usr/bin/env bats
# joy chat list/show reads the git-native .joy/chats (JOY-01F3).

load setup

@test "joy chat list and show render a chat" {
    joy init --name "Chat Test" 2>/dev/null
    run joy chat list
    [ "$status" -eq 0 ]
    [[ "$output" == *"No chats."* ]]

    mkdir -p .joy/chats
    cat > .joy/chats/abc123def456.yaml <<YAML
id: abc123def456
title: Standup
created: 2026-07-04T08:00:00Z
updated: 2026-07-04T08:05:00Z
participants:
  - horst@example.com
  - geordi@example.org
messages:
  - at: 2026-07-04T08:01:00Z
    author: horst@example.com
    text: moin
  - at: 2026-07-04T08:02:00Z
    author: geordi@example.org
    text: hi Horst
YAML
    run joy chat list
    [ "$status" -eq 0 ]
    [[ "$output" == *"abc123def456"* ]]
    [[ "$output" == *"Standup"* ]]

    run joy chat show abc123def456
    [ "$status" -eq 0 ]
    [[ "$output" == *"moin"* ]]
    [[ "$output" == *"hi Horst"* ]]
}
