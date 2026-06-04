#!/usr/bin/env bats
# Decision validity field and replaced_by link (JOY-01B4-65).
# A decision binds when status=closed AND validity=accepted; closing a
# decision defaults validity to accepted; replaced_by implies validity=replaced.

load setup

@test "closing a decision defaults validity to accepted" {
    setup_human_auth
    joy add decision "Use YAML storage"
    ID=$(joy ls 2>/dev/null | grep "Use YAML storage" | awk '{print $1}')
    joy close "$ID"
    run joy show "$ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.validity == "accepted"' >/dev/null
}

@test "an explicit validity is kept when closing (rejected stays rejected)" {
    setup_human_auth
    joy add decision "Adopt microservices"
    ID=$(joy ls 2>/dev/null | grep "Adopt microservices" | awk '{print $1}')
    joy edit "$ID" --validity rejected
    joy close "$ID"
    run joy show "$ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.validity == "rejected"' >/dev/null
}

@test "replaced-by sets the link and implies validity=replaced" {
    setup_human_auth
    joy add decision "Old pillars model"
    OLD=$(joy ls 2>/dev/null | grep "Old pillars model" | awk '{print $1}')
    joy add decision "New pillars model"
    NEW=$(joy ls 2>/dev/null | grep "New pillars model" | awk '{print $1}')
    joy edit "$OLD" --replaced-by "$NEW"
    run joy show "$OLD" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e ".data.replaced_by == \"$NEW\"" >/dev/null
    echo "$output" | jq -e '.data.validity == "replaced"' >/dev/null
}

@test "replaced-by rejects an unknown item ID" {
    setup_human_auth
    joy add decision "Some decision"
    ID=$(joy ls 2>/dev/null | grep "Some decision" | awk '{print $1}')
    run joy edit "$ID" --replaced-by "JOY-FFFF-ZZ"
    [ "$status" -ne 0 ]
}

@test "closing a non-decision leaves validity unset" {
    setup_human_auth
    joy add task "Plain task"
    ID=$(joy ls 2>/dev/null | grep "Plain task" | awk '{print $1}')
    joy close "$ID"
    run joy show "$ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.validity == null' >/dev/null
}

@test "joy show always shows validity for a decision; unset reads as proposed" {
    setup_human_auth
    joy add decision "Still undecided"
    ID=$(joy ls -D 2>/dev/null | grep "Still undecided" | awk '{print $1}')
    run joy show "$ID"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Validity"* ]]
    [[ "$output" == *"proposed"* ]]
}

@test "joy show hides validity for a non-decision" {
    setup_human_auth
    joy add task "Plain show task"
    ID=$(joy ls 2>/dev/null | grep "Plain show task" | awk '{print $1}')
    run joy show "$ID"
    [ "$status" -eq 0 ]
    [[ "$output" != *"Validity"* ]]
}
