#!/usr/bin/env bats
#
# The chat store lets git tidy up (JOY-023C-1E): every chat write is a
# commit through libgit2, which never runs the auto-gc the git binary runs
# after its own commits. Nothing packed or pruned, so a project only grew:
# an operator sandbox reached 39 MB of .git for 0.7 MiB of content, in
# 6140 loose objects and not a single pack.
#
# What is asserted here is the product's part: the write path ASKS git,
# once per process, with --auto. Whether there is anything worth packing
# is git's decision by design, and on a repository this small the honest
# answer is usually no, so asserting a pack would only prove that a test
# managed to reach git's threshold.
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

@test "a chat write asks git to tidy the store" {
    setup_human_auth
    record_git_calls

    joy chat send general "one line" --passphrase "$TEST_PASSPHRASE" >/dev/null

    grep -q -- "gc --auto --quiet" "$GIT_CALLS"
    # and it names the store it means, never the ambient repository
    grep -q -- "--git-dir" "$GIT_CALLS"
}

@test "the check rides along with the writes, not once per lifetime" {
    setup_human_auth
    record_git_calls

    for i in 1 2 3; do
        joy chat send general "line $i" --passphrase "$TEST_PASSPHRASE" >/dev/null
    done

    # every CLI run is its own process and starts the counter at zero, so
    # a long-lived host checks periodically and a shell user every time
    [ "$(grep -c -- "gc --auto" "$GIT_CALLS")" -ge 3 ]
}

@test "a failing git leaves the write alone" {
    setup_human_auth
    # best effort throughout: a git that cannot run must not cost the
    # person their message
    STUB_DIR="$TEST_DIR/stub-bin"
    mkdir -p "$STUB_DIR"
    # only the tidy-up call is broken; everything else is the real git
    cat > "$STUB_DIR/git" <<EOF
#!/bin/sh
case "\$*" in
*"gc --auto"*) exit 127 ;;
esac
exec "$REAL_GIT" "\$@"
EOF
    chmod +x "$STUB_DIR/git"
    export PATH="$STUB_DIR:$PATH"

    run -0 joy chat send general "survives a broken git" --passphrase "$TEST_PASSPHRASE"
    run -0 joy chat show general --passphrase "$TEST_PASSPHRASE"
    [[ "$output" == *"survives a broken git"* ]]
}
