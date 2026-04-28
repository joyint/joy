#!/usr/bin/env bats
# Verify that human-readable author fields on items (Comment.author,
# Item.created_by) record the full delegation chain when an AI acts
# under delegation, matching what the event log records.
#
# The doctrine (vision/trustship/Auth.md, vision/traceability/Judge.md)
# requires `[ai:claude@joy delegated-by:human@...]` wherever a human
# reads attribution. Assignees stay member-only because they name a
# concrete responsible actor, not an audit attribution.

load setup

@test "Comment.author includes delegated-by when AI acts via delegation" {
    setup_human_auth
    joy add task "Item under delegation"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Item under delegation" | awk '{print $1}')
    setup_ai_session ai:test@joy
    joy comment "$ITEM_ID" "AI comment"

    grep -q "author: ai:test@joy delegated-by:test@example.com" \
        .joy/items/${ITEM_ID}-*.yaml
}

@test "Item.created_by includes delegated-by when AI creates" {
    setup_human_auth
    setup_ai_session ai:test@joy
    joy add task "AI created"

    grep -q "created_by: ai:test@joy delegated-by:test@example.com" \
        .joy/items/*.yaml
}

@test "Comment.author is bare member when human acts directly" {
    setup_human_auth
    joy add task "Direct human"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Direct human" | awk '{print $1}')
    joy comment "$ITEM_ID" "Human comment"

    # Should be exactly the email, with no delegated-by suffix.
    grep -E "^- author: test@example.com\$" .joy/items/${ITEM_ID}-*.yaml
}

@test "Assignees remain member-only under delegation" {
    setup_human_auth
    joy project member add ai:test@joy --passphrase "$TEST_PASSPHRASE"
    setup_ai_session ai:test@joy
    joy add task "Assignment test"
    ITEM_ID=$(joy ls 2>/dev/null | grep "Assignment test" | awk '{print $1}')
    joy assign "$ITEM_ID"

    # Assignee.member must NOT carry delegated-by.
    grep -E "^- member: ai:test@joy\$" .joy/items/${ITEM_ID}-*.yaml
    ! grep "member: ai:test@joy delegated-by" .joy/items/${ITEM_ID}-*.yaml
}
