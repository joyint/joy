#!/usr/bin/env bash
# Regenerates the member fixtures (JOY-02C2-F3): two small projects in the
# .joy format of joy 0.21, one open and one anonymous, each with a founder,
# a second person, an AI member with its delegation, and an issued token.
#
# The fixtures are checked in and stay in THIS format on purpose: they are
# what a project looked like before the member files, and every later
# version has to open them. Run this only to add a case, with the joy
# binary of the version the fixtures stand for:
#
#   JOY_BIN=target/debug/joy tests/fixtures/members/generate.sh
#
# A debug binary derives keys with the cheap test parameters, which is
# what the tests that read the fixtures run with.
set -euo pipefail

JOY="$(realpath "${JOY_BIN:?set JOY_BIN to a debug joy binary}")"
OUT="$(cd "$(dirname "$0")" && pwd)"

FOUNDER="founder@example.com"
SECOND="second@example.com"
FOUNDER_PASS="correct horse battery staple extra words"
SECOND_PASS="alpha bravo charlie delta echo foxtrot"
AI="ai:claude@joy"
# Ten years: a fixture token must not expire under the tests.
TTL_HOURS=87600

build() {
    local name="$1" privacy="$2"
    local work
    work="$(mktemp -d)"
    (
        cd "$work"
        export HOME="$work/home" XDG_STATE_HOME="$work/home/.state" XDG_CONFIG_HOME="$work/home/.config"
        export GIT_CONFIG_NOSYSTEM=1
        mkdir -p "$HOME" project && cd project
        git init --quiet
        git config user.email "$FOUNDER"
        git config user.name "Founder"

        "$JOY" init --name "Fixture $name" --acronym FX >/dev/null 2>&1
        "$JOY" auth init --passphrase "$FOUNDER_PASS" >/dev/null
        otp=$("$JOY" project member add "$SECOND" --passphrase "$FOUNDER_PASS" \
            | grep -oE '[A-Z0-9]{4}-[A-Z0-9]{4}-[A-Z0-9]{4}' | head -1)
        "$JOY" auth --otp "$otp" --user "$SECOND" --passphrase "$SECOND_PASS" >/dev/null
        "$JOY" auth --user "$FOUNDER" --passphrase "$FOUNDER_PASS" >/dev/null
        "$JOY" ai init --tool claude --passphrase "$FOUNDER_PASS" >/dev/null 2>&1
        "$JOY" add task "A task from before the member files" >/dev/null
        if [ "$privacy" = anonymous ]; then
            "$JOY" project set privacy anonymous --passphrase "$FOUNDER_PASS" >/dev/null
        fi
        "$JOY" auth token add "$AI" --ttl "$TTL_HOURS" --passphrase "$FOUNDER_PASS" \
            | tr -d '"' > "$work/token"
    )
    rm -rf "$OUT/$name"
    mkdir -p "$OUT/$name"
    # Only what a clone carries: the committed part of .joy.
    (cd "$work/project" && git add -A .joy && git ls-files .joy) | while read -r f; do
        mkdir -p "$OUT/$name/dot-joy/$(dirname "${f#.joy/}")"
        cp "$work/project/$f" "$OUT/$name/dot-joy/${f#.joy/}"
    done
    cp "$work/token" "$OUT/$name/ai-token"
    rm -rf "$work"
    echo "wrote $OUT/$name"
}

build open open
build anonymous anonymous
