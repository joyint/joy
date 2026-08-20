#!/usr/bin/env bats
#
# Two writers on one chat ref (JOY-023B-7E): chats live on refs/joy/chats,
# and a write is read-tip, build, move-ref. Without a compare-and-swap the
# loser silently overwrites the winner, so messages vanish from a chat
# that looks healthy and the overwritten commits linger unreachable.
#
# Nothing is faked here: real joy, real git, real parallel processes.

load setup

MESSAGES=12

@test "parallel sends never lose a message silently" {
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
    local sent=0
    for i in $(seq 1 $MESSAGES); do
        if [ "$(cat "$TEST_DIR/rc-$i")" = "0" ]; then
            # a send that reported success IS in the chat
            grep -q "msg-$i-end" "$TEST_DIR/shown" || {
                echo "msg-$i-end reported success but is missing" >&2
                false
            }
            sent=$((sent + 1))
        else
            # and a send that lost the race says so, in the words a
            # person can act on
            grep -q "try again" "$TEST_DIR/out-$i"
            run -1 grep -q "msg-$i-end" "$TEST_DIR/shown"
        fi
    done
    # at least one writer got through, or the run proves nothing. How
    # MANY survive depends on machine load, so it is deliberately not
    # asserted: what is under test is that failure is never silent.
    [ "$sent" -ge 1 ]

    # (a writer that lost the race has already built its commit; that
    # object stays behind unreachable by design and is what the
    # maintenance pass of JOY-023C-1E collects, so it is not asserted
    # here)
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
