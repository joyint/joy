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

loose_objects() {
    find .git/objects -type f -path '*/??/*' | grep -v '/17/' || true
}

packs() {
    find .git/objects/pack -name '*.pack' 2>/dev/null || true
}

@test "a chat write never asks a git process to tidy the store" {
    setup_human_auth
    record_git_calls

    run -0 joy chat send general "one line" --passphrase "$TEST_PASSPHRASE"

    # the spawn that used to stand here, in every shape it could take
    run -1 grep -q -- "gc" "$GIT_CALLS"
    # The store's own crate spawns nothing at all any more. What is left
    # on the chat path is ONE call from joy-cli, the remote probe
    # (joy_core::vcs::remote_exists), a named CLI-git helper that D3.2
    # moves to git2 in J6. Pinned here so the count cannot grow quietly.
    calls="$(sort -u "$GIT_CALLS")"
    [ "$(printf '%s\n' "$calls" | grep -c .)" -le 1 ]
    [[ -z "$calls" || "$calls" == *"remote get-url"* ]]
    # and the message is there
    run -0 joy chat show general --passphrase "$TEST_PASSPHRASE"
    [[ "$output" == *"one line"* ]]
}

@test "a store worth packing is packed by the write itself, without git" {
    setup_human_auth
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

    # THIS shell is the second process: it holds a loose object open
    # across the write that packs and sweeps.
    held="$(loose_objects | head -1)"
    [ -n "$held" ]
    size="$(wc -c < "$held")"
    exec 9< "$held"

    run -0 joy chat send general "during the sweep" --passphrase "$TEST_PASSPHRASE"

    # the held descriptor still reads the whole object, whether or not
    # the loose copy was swept into the pack
    [ "$(wc -c <&9)" -eq "$size" ]
    exec 9<&-

    # and every chat is intact, read through whatever backend holds the
    # objects now
    run -0 joy chat show general --passphrase "$TEST_PASSPHRASE"
    [[ "$output" == *"before the sweep"* ]]
    [[ "$output" == *"during the sweep"* ]]
}
