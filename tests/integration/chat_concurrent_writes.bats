#!/usr/bin/env bats
#
# Twelve writers on one chat ref (JOY-023B-7E, JOY-02AB-F3): chats live on
# refs/joy/chats, and a write is read-tip, build, move-ref. Writers on one
# checkout queue behind the store's write lock, so every send lands and
# nobody is told to try again; the compare-and-swap underneath stays as
# the safety net, so a message can still never vanish silently.
#
# Nothing is faked here: real joy, real git, real parallel processes.

load setup

MESSAGES=12

@test "parallel sends all land and none is refused" {
    setup_human_auth

    for i in $(seq 1 $MESSAGES); do
        # set +e: a refusal must reach the rc file, not abort the subshell
        ( set +e
          joy chat send general "msg-$i-end" --passphrase "$TEST_PASSPHRASE" \
              > "$TEST_DIR/out-$i" 2>&1
          echo "$?" > "$TEST_DIR/rc-$i" ) &
    done
    wait

    joy chat show general --passphrase "$TEST_PASSPHRASE" > "$TEST_DIR/shown"
    for i in $(seq 1 $MESSAGES); do
        # every send succeeds: the writers queue, they do not race
        if [ "$(cat "$TEST_DIR/rc-$i")" != "0" ]; then
            echo "send $i failed:" >&2
            cat "$TEST_DIR/out-$i" >&2
            false
        fi
        # and nobody was asked to do the person's work again
        run -1 grep -q "try again" "$TEST_DIR/out-$i"
        # and what reported success IS in the chat
        grep -q "msg-$i-end" "$TEST_DIR/shown" || {
            echo "msg-$i-end reported success but is missing" >&2
            false
        }
    done
}

@test "a second writer folds onto the winner instead of replacing it" {
    setup_human_auth
    joy chat send general "first line" --passphrase "$TEST_PASSPHRASE" >/dev/null

    # two writers start from the same tip
    joy chat send general "left branch" --passphrase "$TEST_PASSPHRASE" >/dev/null 2>&1 &
    joy chat send general "right branch" --passphrase "$TEST_PASSPHRASE" >/dev/null 2>&1 &
    wait

    run -0 joy chat show general --passphrase "$TEST_PASSPHRASE"
    [[ "$output" == *"first line"* ]]
    [[ "$output" == *"left branch"* ]]
    [[ "$output" == *"right branch"* ]]
}
