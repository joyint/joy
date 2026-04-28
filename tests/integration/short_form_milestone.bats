#!/usr/bin/env bats
# Verify that joy milestone link with a short-form milestone ID is
# normalized to the full form at load time, so joy roadmap shows the
# correct bucket title and milestone filters work.

load setup

@test "joy roadmap shows real milestone title when linked via short-form ID" {
    joy init --name "Test" --acronym TST
    MS_FULL=$(joy milestone add "First Sprint" 2>&1 \
        | grep -oE 'TST-MS-[0-9A-F]+(-[0-9A-F]+)?' | head -1)
    MS_SHORT="${MS_FULL%-*}"

    joy add task "Item"
    ITEM=$(joy ls 2>/dev/null | grep "Item" | awk '{print $1}')
    joy milestone link "$ITEM" "$MS_SHORT"

    run joy roadmap
    [ "$status" -eq 0 ]
    [[ "$output" == *"First Sprint"* ]]
    [[ "$output" != *"(undefined)"* ]]
}

@test "short-form milestone ref is persisted in full form on next write" {
    joy init --name "Test" --acronym TST
    MS_FULL=$(joy milestone add "First Sprint" 2>&1 \
        | grep -oE 'TST-MS-[0-9A-F]+(-[0-9A-F]+)?' | head -1)
    MS_SHORT="${MS_FULL%-*}"

    joy add task "Item"
    ITEM=$(joy ls 2>/dev/null | grep "Item" | awk '{print $1}')
    joy milestone link "$ITEM" "$MS_SHORT"

    # Right after link, YAML still holds the short form
    grep -q "^milestone: ${MS_SHORT}\$" .joy/items/${ITEM}-*.yaml

    # Any subsequent edit must persist the full form
    joy edit "$ITEM" --priority high
    grep -q "^milestone: ${MS_FULL}\$" .joy/items/${ITEM}-*.yaml
    ! grep -q "^milestone: ${MS_SHORT}\$" .joy/items/${ITEM}-*.yaml
}

@test "joy ls --milestone matches items linked by short-form ID" {
    joy init --name "Test" --acronym TST
    MS_FULL=$(joy milestone add "First Sprint" 2>&1 \
        | grep -oE 'TST-MS-[0-9A-F]+(-[0-9A-F]+)?' | head -1)
    MS_SHORT="${MS_FULL%-*}"

    joy add task "Linked"
    ITEM=$(joy ls 2>/dev/null | grep "Linked" | awk '{print $1}')
    joy milestone link "$ITEM" "$MS_SHORT"

    run joy ls --milestone "$MS_FULL"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Linked"* ]]
}
