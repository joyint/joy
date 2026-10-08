#!/usr/bin/env bash
# Records a project with a team chat the way joy 0.22 stored one, as a git
# bundle: project.yaml with the AI member, a person's delegation to it,
# and on refs/joy/chats a sealed chat with that AI member as participant,
# a line of the person, an answer of the AI member and its session.
#
# joy 0.22 wrote an AI member in the legacy form. Every later version has
# to open what it wrote and name the member by its name, in every store.
# The member fixtures next door hold what lives in .joy; a chat lives in
# a git ref, so it needs a bundle of its own. A double answer on
# integration (2026-10-08) came from exactly this gap: no test ever
# opened a chat that an earlier release had written.
#
# The bundles are checked in and stay as they are. Run this only to add a
# case, against a checkout of the release they stand for, built with a
# small recorder that calls that release's own chat functions:
#
#   git worktree add --detach /tmp/before/joy v0.22.0     (and its crypt beside it)
#   cp tests/fixtures/chat-from-before/record_chat.rs \
#      /tmp/before/joy/crates/joy-chat-store/examples/
#   (cd /tmp/before/joy && cargo build -p joy-cli \
#       && cargo build -p joy-chat-store --example record_chat)
#   BEFORE=/tmp/before/joy/target/debug tests/fixtures/chat-from-before/record.sh \
#       founder@example.com tests/fixtures/chat-from-before/project.bundle
#
# A debug build derives keys with the cheap test parameters, which is what
# the tests that open the bundle run with. A release build (target/release)
# derives them the way a person's machine does: that is the bundle the
# app's browser tests open.
set -euo pipefail

BEFORE="$(realpath "${BEFORE:?set BEFORE to the target dir of the built release}")"
OWNER="${1:?the person: an address}"
OUT="$(realpath -m "${2:?where the bundle goes}")"
JOY="$BEFORE/joy"
RECORD="$BEFORE/examples/record_chat"
for built in "$JOY" "$RECORD"; do
    [ -x "$built" ] || { echo "not built: $built" >&2; exit 1; }
done
PASSPHRASE="correct horse battery staple"
AI="ai:vibe@joy"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
(
    cd "$work"
    export HOME="$work/home" XDG_STATE_HOME="$work/home/.state" XDG_CONFIG_HOME="$work/home/.config"
    export GIT_CONFIG_NOSYSTEM=1
    mkdir -p "$HOME" project && cd project
    git init --quiet -b main
    git config user.email "$OWNER"
    git config user.name "Owner"
    "$JOY" init --name "Chat from before" --acronym CB >/dev/null 2>&1
    "$JOY" auth init --passphrase "$PASSPHRASE" >/dev/null
    "$JOY" project member add "$AI" --with-token --passphrase "$PASSPHRASE" >/dev/null 2>&1
    git add -A
    git commit --quiet -m "a project from before [no-item]"
    "$RECORD" . "$OWNER" "$PASSPHRASE" "$AI" >/dev/null
    git bundle create --quiet "$OUT" --all
)
echo "wrote $OUT"
