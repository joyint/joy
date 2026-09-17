#!/usr/bin/env bats
#
# The chat store maintains itself, with git2 and no git process (design
# D3.7). Every chat write is a commit through libgit2, which never runs
# the auto-gc the git binary runs after its own commits, so a project only
# grew: an operator sandbox reached 39 MB of .git for 0.7 MiB of content,
# in 6140 loose objects, 72 percent of them unreachable, and not a single
# pack.
#
# What used to be asserted here was that the write path SPAWNS
# `git gc --auto`. That spawn is gone: it broke the git2-only rule, it is
# a hazard beside a shallow checkout, and packing alone was never the
# answer to unreachable objects anyway. joy packs and sweeps in process
# instead, and these cases assert the observable half of it: no git
# process is asked to tidy the store, a store worth packing is packed by
# the write itself, the chats survive it, and a second process holding an
# object open changes none of that.
#
# The `git` on PATH is a marked RECORDING WRAPPER: it notes the call and
# then runs the real git, so nothing about the store is faked.

load setup

REAL_GIT="$(command -v git)"

record_git_calls() {
    STUB_DIR="$TEST_DIR/stub-bin"
    mkdir -p "$STUB_DIR"
    GIT_CALLS="$TEST_DIR/git-calls"
    : > "$GIT_CALLS"
    cat > "$STUB_DIR/git" <<EOF
#!/bin/sh
echo "\$*" >> "$GIT_CALLS"
exec "$REAL_GIT" "\$@"
EOF
    chmod +x "$STUB_DIR/git"
    export PATH="$STUB_DIR:$PATH"
}

# Make the store look worth maintaining. The trigger is git's own cheap
# estimate: count one fanout directory and multiply by 256, so 27 entries
# in objects/17 are 6912 estimated loose objects, past git's threshold of
# 6700. Filling a store with 6700 real objects would cost minutes and
# prove the same thing.
make_the_store_look_full() {
    mkdir -p .git/objects/17
    for i in $(seq 0 26); do
        printf '' > ".git/objects/17/$(printf '%038x' "$i")"
    done
}

# The loose file of one object, by id.
loose_path() {
    echo ".git/objects/${1:0:2}/${1:2}"
}

# A remote the chat write path can actually push to, on the local
# transport: without one the push branch of `joy chat send` is never
# reached and a case that counts git processes counts nothing.
add_local_remote() {
    git init --quiet --bare "$TEST_DIR/remote.git"
    git remote add origin "$TEST_DIR/remote.git"
}

packs() {
    find .git/objects/pack -name '*.pack' 2>/dev/null || true
}

@test "a chat write never asks a git process to tidy the store" {
    setup_human_auth
    # WITH a remote, so the whole write path runs: the store write, the
    # maintenance and the delivery. Without one the push is unreachable
    # and the count below would be a pin on nothing.
    add_local_remote
    record_git_calls

    run -0 joy chat send general "one line" --passphrase "$TEST_PASSPHRASE"

    # the spawn that used to stand here, in every shape it could take
    run -1 grep -q -- "gc" "$GIT_CALLS"
    # NOTHING on the chat write path spawns git any more: not the store,
    # not the maintenance, not the delivery push. J7 had to leave the
    # push behind because a transport needs joy-core's `forge-net`
    # feature and joy-cli did not enable it; J6 enables it (D3.1) and
    # moves the transfer onto the engine (D3.2), so the count this case
    # pins is ZERO. The `remote get-url` probe that stood beside it is
    # git2 too.
    [ ! -s "$GIT_CALLS" ]
    # ...and the delivery really happened, so the count above is a pin
    # on a path that ran and not on one that was skipped.
    run -0 git --git-dir="$TEST_DIR/remote.git" rev-parse refs/joy/chats
    # and the message is there
    run -0 joy chat show general --passphrase "$TEST_PASSPHRASE"
    [[ "$output" == *"one line"* ]]
}

@test "a store worth packing is packed by the write itself, without git" {
    setup_human_auth
    add_local_remote
    record_git_calls
    make_the_store_look_full
    [ -z "$(packs)" ]

    run -0 joy chat send general "the write that packs" --passphrase "$TEST_PASSPHRASE"

    [ -n "$(packs)" ]
    run -1 grep -q -- "gc" "$GIT_CALLS"
    run -0 joy chat show general --passphrase "$TEST_PASSPHRASE"
    [[ "$output" == *"the write that packs"* ]]
}

@test "the ten minute floor means the next write packs nothing again" {
    setup_human_auth
    make_the_store_look_full

    joy chat send general "first" --passphrase "$TEST_PASSPHRASE" >/dev/null
    joy chat send general "second" --passphrase "$TEST_PASSPHRASE" >/dev/null

    # every CLI command is its own process, so the floor lives on disk;
    # without it a busy store would collect one small pack per write
    [ "$(packs | wc -l)" -eq 1 ]
    run -0 joy chat show general --passphrase "$TEST_PASSPHRASE"
    [[ "$output" == *"first"* ]]
    [[ "$output" == *"second"* ]]
}

@test "a second process holding an object open does not break the sweep" {
    setup_human_auth
    joy chat send general "before the sweep" --passphrase "$TEST_PASSPHRASE" >/dev/null
    make_the_store_look_full

    # THIS shell is the second process, and the object it holds open is
    # the chat tip: reachable, so the sweep packs it and removes the
    # loose copy (class A). Picking any loose file would prove nothing,
    # because an orphan inside the 14 day window is not touched at all.
    tip="$(git rev-parse refs/joy/chats)"
    held="$(loose_path "$tip")"
    [ -f "$held" ]
    size="$(wc -c < "$held")"
    exec 9< "$held"

    run -0 joy chat send general "during the sweep" --passphrase "$TEST_PASSPHRASE"

    # the loose copy is gone: the sweep really did remove a file this
    # shell had open
    [ ! -e "$held" ]
    [ -n "$(packs)" ]
    # the held descriptor still reads the whole object, which is what an
    # unlink under a live reader means on unix
    [ "$(wc -c <&9)" -eq "$size" ]
    exec 9<&-
    # and every reader finds the object again, out of the pack
    run -0 git cat-file -e "$tip"

    # and every chat is intact, read through whatever backend holds the
    # objects now
    run -0 joy chat show general --passphrase "$TEST_PASSPHRASE"
    [[ "$output" == *"before the sweep"* ]]
    [[ "$output" == *"during the sweep"* ]]
}
