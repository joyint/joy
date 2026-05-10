#!/usr/bin/env bats
# End-to-end tests for the YAML-aware Git merge driver (JOY-00B0).
# Exercises the actual git merge path: gitattributes pattern matching,
# driver invocation, encrypted-blob handling, and log union.

load setup

@test "joy init writes .gitattributes block and registers merge driver" {
    joy init --name "Test"
    [ -f ".gitattributes" ]
    grep -q "merge=joy-yaml" .gitattributes
    grep -q "merge=union" .gitattributes
    [ "$(git config --local merge.joy-yaml.name)" = "Joy YAML merge driver" ]
    git config --local merge.joy-yaml.driver | grep -q "joy merge driver"
    git config --local merge.joy-yaml.driver | grep -q -- "--ours-rev %X"
    git config --local merge.joy-yaml.driver | grep -q -- "--theirs-rev %Y"
}

@test "merge of conflicting field changes resolves to single 'updated' line and keeps both edits" {
    joy init --name "Test"
    joy add task "Mergeable" --effort 2
    git add -A && git commit -m "init [no-item]" --quiet
    ITEM_ID=$(joy ls 2>/dev/null | grep "Mergeable" | awk '{print $1}')
    ITEM_FILE=$(ls .joy/items/${ITEM_ID}*.yaml | head -1)
    BRANCH=$(git symbolic-ref --short HEAD)

    git checkout -q -b branch-a
    joy edit "$ITEM_ID" --priority high
    git add -A && git commit -m "branch a [no-item]" --quiet

    git checkout -q "$BRANCH"
    git checkout -q -b branch-b
    sleep 1
    joy edit "$ITEM_ID" --tags integration
    git add -A && git commit -m "branch b [no-item]" --quiet

    run git merge --no-edit -q branch-a
    [ "$status" -eq 0 ]

    [ "$(grep -c '^updated:' "$ITEM_FILE")" -eq 1 ]
    grep -q '^priority: high' "$ITEM_FILE"
    grep -q 'integration' "$ITEM_FILE"
    joy show "$ITEM_ID" >/dev/null
}

@test "comments added on both branches are unioned after merge" {
    joy init --name "Test"
    joy add task "Comment Mergeable" --effort 2
    git add -A && git commit -m "init [no-item]" --quiet
    ITEM_ID=$(joy ls 2>/dev/null | grep "Comment Mergeable" | awk '{print $1}')
    BRANCH=$(git symbolic-ref --short HEAD)

    git checkout -q -b branch-a
    joy comment "$ITEM_ID" "comment from a"
    git add -A && git commit -m "branch a [no-item]" --quiet

    git checkout -q "$BRANCH"
    git checkout -q -b branch-b
    sleep 1
    joy comment "$ITEM_ID" "comment from b"
    git add -A && git commit -m "branch b [no-item]" --quiet

    run git merge --no-edit -q branch-a
    [ "$status" -eq 0 ]

    run joy show "$ITEM_ID"
    [[ "$output" == *"comment from a"* ]]
    [[ "$output" == *"comment from b"* ]]
}

@test "log files merge cleanly via union, no conflict on same-day appends" {
    joy init --name "Test"
    joy add task "Initial" --effort 2
    git add -A && git commit -m "init [no-item]" --quiet
    BRANCH=$(git symbolic-ref --short HEAD)

    git checkout -q -b branch-a
    A_ID=$(joy add task "From branch a" --effort 2 | sed -n 's/^Created \([A-Z0-9-]\+\) .*/\1/p')
    git add -A && git commit -m "branch a [no-item]" --quiet

    git checkout -q "$BRANCH"
    git checkout -q -b branch-b
    sleep 1
    B_ID=$(joy add task "From branch b" --effort 2 | sed -n 's/^Created \([A-Z0-9-]\+\) .*/\1/p')
    git add -A && git commit -m "branch b [no-item]" --quiet

    run git merge --no-edit -q branch-a
    [ "$status" -eq 0 ]

    # Titles are not in the log (JOY-0175-9B); both per-branch
    # creation events must survive the union merge by ID.
    grep -hq "$A_ID item.created" .joy/logs/*.log
    grep -hq "$B_ID item.created" .joy/logs/*.log
    # No conflict markers leaked into any log file.
    ! grep -lq '^<<<<<<< ' .joy/logs/*.log 2>/dev/null
}

@test "JOYCRYPT blob is never byte-merged; later commit timestamp wins" {
    joy init --name "Test"
    git add -A && git commit -m "init [no-item]" --quiet
    BRANCH=$(git symbolic-ref --short HEAD)

    # Plant a fake JOYCRYPT-prefixed file at a path that matches merge=joy-yaml.
    # Layout: 'JOYCRYPT' + 0x01 (version) + 0x07 (zone-len) + 'default' + payload.
    printf 'JOYCRYPT\x01\x07defaultBASE_PAYLOAD' > .joy/items/JOY-FAKE-EN.yaml
    git add -A && git commit -m "fake encrypted base [no-item]" --quiet

    # branch-b: commit first, so its timestamp is older.
    git checkout -q -b branch-b
    printf 'JOYCRYPT\x01\x07defaultBRANCH_B_PAYLOAD' > .joy/items/JOY-FAKE-EN.yaml
    git add -A && git commit -m "branch b payload [no-item]" --quiet

    git checkout -q "$BRANCH"
    git checkout -q -b branch-a
    sleep 1
    printf 'JOYCRYPT\x01\x07defaultBRANCH_A_PAYLOAD' > .joy/items/JOY-FAKE-EN.yaml
    git add -A && git commit -m "branch a payload [no-item]" --quiet

    git checkout -q branch-b
    run git merge --no-edit -q branch-a
    [ "$status" -eq 0 ]

    # ours=branch-b (older), theirs=branch-a (newer) -> driver picks theirs.
    grep -aq "BRANCH_A_PAYLOAD" .joy/items/JOY-FAKE-EN.yaml
    ! grep -aq "BRANCH_B_PAYLOAD" .joy/items/JOY-FAKE-EN.yaml
    # And the driver did not concatenate or insert YAML / conflict markers.
    ! grep -aq '<<<<<<<' .joy/items/JOY-FAKE-EN.yaml
}
