#!/usr/bin/env bats
# JSON output for joy release ls/show (JOY-012E-B9, ADR-036 §1).

load setup

@test "joy release ls --json on no releases returns empty list" {
    setup_human_auth
    run joy release ls --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.version == 1' >/dev/null
    echo "$output" | jq -e '.data.total == 0' >/dev/null
    echo "$output" | jq -e '.data.releases == []' >/dev/null
}

@test "joy release show --json on preview returns previous_version + closed_item_ids" {
    setup_human_auth
    run joy release show --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data | has("previous_version")' >/dev/null
    echo "$output" | jq -e '.data | has("closed_item_ids")' >/dev/null
}
