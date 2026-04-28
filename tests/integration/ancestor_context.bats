#!/usr/bin/env bats
# Verify that joy ls --tree and joy roadmap pull ancestors of primary
# items into the visible set as context, so the parent-child hierarchy
# stays connected when ancestors are filtered out (closed/deferred) or
# live in a different milestone bucket.

load setup

@test "joy ls --tree shows closed parent above active child" {
    joy init --name "Test" --acronym TST
    joy add task "Parent A"
    PARENT=$(joy ls 2>/dev/null | grep "Parent A" | awk '{print $1}')
    joy add task "Child B" --parent "$PARENT"
    CHILD=$(joy ls 2>/dev/null | grep "Child B" | awk '{print $1}')
    joy close "$PARENT"

    # Without --all the closed parent is not a primary item, yet must
    # still appear in the tree above its active child.
    run joy ls --tree
    [ "$status" -eq 0 ]
    [[ "$output" == *"Parent A"* ]]
    [[ "$output" == *"Child B"* ]]
    # Child must be rendered indented under Parent (tree connector before child).
    echo "$output" | grep -E "(└──|├──).*Child B" >/dev/null
}

@test "joy ls --tree shows full ancestor chain when intermediate is filtered" {
    joy init --name "Test" --acronym TST
    joy add task "Grand"
    GRAND=$(joy ls 2>/dev/null | grep "Grand" | awk '{print $1}')
    joy add task "Mid" --parent "$GRAND"
    MID=$(joy ls 2>/dev/null | grep "Mid" | awk '{print $1}')
    joy add task "Leaf" --parent "$MID"
    joy close "$MID"

    run joy ls --tree
    [ "$status" -eq 0 ]
    # All three names visible despite Mid being closed.
    [[ "$output" == *"Grand"* ]]
    [[ "$output" == *"Mid"* ]]
    [[ "$output" == *"Leaf"* ]]
    # Leaf must be deeper than Mid (more indentation).
    GRAND_DEPTH=$(echo "$output" | grep "Grand" | head -1 | sed 's/[^ ].*//' | wc -c)
    LEAF_DEPTH=$(echo "$output" | grep "Leaf" | head -1 | sed 's/[^ ].*//' | wc -c)
    [ "$LEAF_DEPTH" -gt "$GRAND_DEPTH" ]
}

@test "joy roadmap shows parent as context in child's milestone bucket" {
    joy init --name "Test" --acronym TST
    MS01=$(joy milestone add "First" 2>&1 | grep -oE 'TST-MS-[0-9A-F]+(-[0-9A-F]+)?' | head -1)
    MS02=$(joy milestone add "Second" 2>&1 | grep -oE 'TST-MS-[0-9A-F]+(-[0-9A-F]+)?' | head -1)

    joy add epic "Parent in MS-01"
    PARENT=$(joy ls 2>/dev/null | grep "Parent in MS-01" | awk '{print $1}')
    joy milestone link "$PARENT" "$MS01"

    joy add task "Child in MS-02" --parent "$PARENT"
    CHILD=$(joy ls 2>/dev/null | grep "Child in MS-02" | awk '{print $1}')
    joy milestone link "$CHILD" "$MS02"

    run joy roadmap
    [ "$status" -eq 0 ]

    # In the MS-02 section, the child must appear under its parent,
    # not as a top-level root.
    MS02_SECTION=$(echo "$output" | awk "/${MS02} Second/,/^\$|^---/")
    echo "$MS02_SECTION" | grep "Parent in MS-01" >/dev/null
    echo "$MS02_SECTION" | grep -E "(└──|├──).*Child in MS-02" >/dev/null
}

@test "joy roadmap counters reflect primary items only, not context ancestors" {
    joy init --name "Test" --acronym TST
    MS01=$(joy milestone add "First" 2>&1 | grep -oE 'TST-MS-[0-9A-F]+(-[0-9A-F]+)?' | head -1)
    MS02=$(joy milestone add "Second" 2>&1 | grep -oE 'TST-MS-[0-9A-F]+(-[0-9A-F]+)?' | head -1)

    joy add epic "Parent"
    PARENT=$(joy ls 2>/dev/null | grep "Parent" | awk '{print $1}')
    joy milestone link "$PARENT" "$MS01"

    joy add task "Child" --parent "$PARENT"
    CHILD=$(joy ls 2>/dev/null | grep "Child" | awk '{print $1}')
    joy milestone link "$CHILD" "$MS02"

    run joy roadmap
    [ "$status" -eq 0 ]

    # MS-02 has only one primary item (the child).
    # The parent appears as context but must not be counted in the [n/m] header.
    echo "$output" | grep -E "${MS02} Second \[0/1\]" >/dev/null
}
