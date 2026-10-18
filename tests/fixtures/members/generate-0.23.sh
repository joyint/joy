#!/usr/bin/env bash
# Regenerates the member fixtures of joy 0.23 (JOY-02DB-09): the two
# projects of generate.sh, brought over to member files by joy 0.23
# itself, with a yes of the founder at a terminal. They are what a project
# looked like after that update and before format 4: one file per member,
# `email` or `name` at the top, `seed_wrap_passphrase`, project.yaml at
# format 2.
#
# The fixtures are checked in and stay in THIS format on purpose, like the
# ones they are made from. Run this only to add a case, with a debug joy
# binary of version 0.23:
#
#   JOY_BIN=<checkout of 0.23>/target/debug/joy tests/fixtures/members/generate-0.23.sh
set -euo pipefail

JOY="$(realpath "${JOY_BIN:?set JOY_BIN to a debug joy 0.23 binary}")"
OUT="$(cd "$(dirname "$0")" && pwd)"

FOUNDER="founder@example.com"
FOUNDER_PASS="correct horse battery staple extra words"

build() {
    local name="$1"
    local work
    work="$(mktemp -d)"
    (
        export HOME="$work/home" XDG_STATE_HOME="$work/home/.state" XDG_CONFIG_HOME="$work/home/.config"
        export GIT_CONFIG_NOSYSTEM=1
        mkdir -p "$HOME" "$work/project"
        cp -r "$OUT/$name/dot-joy" "$work/project/.joy"
        cd "$work/project"
        git init --quiet
        git config user.email "$FOUNDER"
        git config user.name "Founder"
        # the yes, the founder's passphrase, and no commit
        printf 'y\n%s\nn\n' "$FOUNDER_PASS" | script -qec "$JOY update --no-binary" /dev/null >/dev/null
        test -d .joy/members
    )
    rm -rf "$OUT/$name-0.23"
    mkdir -p "$OUT/$name-0.23"
    (cd "$work/project" && git add -A .joy && git ls-files .joy) | while read -r f; do
        mkdir -p "$OUT/$name-0.23/dot-joy/$(dirname "${f#.joy/}")"
        cp "$work/project/$f" "$OUT/$name-0.23/dot-joy/${f#.joy/}"
    done
    cp "$OUT/$name/ai-token" "$OUT/$name-0.23/ai-token"
    rm -rf "$work"
    echo "wrote $OUT/$name-0.23"
}

build open
build anonymous
