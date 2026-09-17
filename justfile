# Joy -- Task Runner
# See docs/dev/CONTRIBUTING.md for full documentation

# Load `.env` so release recipes can read credentials like
# CARGO_REGISTRY_TOKEN from the environment. just searches from the
# working directory upward and uses the closest file, so a local
# `joy/.env` wins and otherwise the umbrella's `.env` is picked up.
# This makes `just publish` / `just release` work when run directly
# inside this submodule, not only through the umbrella's release-all.
set dotenv-load

# List recipes
default:
    @just --list

# Run all tests (unit + snapshot + integration)
test: test-unit test-cmd test-int

# Rust tests: lib AND integration tests (crates/*/tests/*.rs) — matches
# what CI runs (cargo test --workspace). Do NOT re-add --lib: it silently
# skips the integration tests (e.g. job_session_mint) and lets a red CI
# slip past a green local check. fast-kdf keeps Argon2id minimal for speed.
test-unit:
    cargo test --workspace --features fast-kdf

# Snapshot tests (trycmd)
test-cmd:
    cargo test -p joy-cli --test cmd --features fast-kdf

# Integration tests (bats)
test-int:
    cargo build -p joy-cli -p joy-github -p joy-gitlab -p joy-gitea --features joy-cli/fast-kdf
    bats tests/integration/*.bats

# The functional core, in seconds: the handful of bats cases tagged
# `smoke` (item lifecycle, authentication, a guard refusal, the chat
# lifecycle, forge identity resolution). `just check` runs these so a
# commit is gated on the product still WORKING, not only on it
# compiling; the full suite rides in `just check-all`.
test-smoke:
    cargo build -p joy-cli -p joy-github -p joy-gitlab -p joy-gitea --features joy-cli/fast-kdf
    bats --filter-tags smoke tests/integration/*.bats

# Snapshot tests (insta)
test-snap:
    cargo insta test --workspace

# Update snapshots
test-snap-update:
    cargo insta test --workspace --review

# Coverage report (terminal summary)
test-coverage:
    cargo llvm-cov --workspace

# Coverage report (HTML, opens in browser)
test-coverage-html:
    cargo llvm-cov --workspace --html --open

# Re-run tests on change
test-watch:
    cargo watch -x 'test --workspace'

# Format all code
fmt:
    cargo fmt --all

# Check formatting
fmt-check:
    cargo fmt --all -- --check

# Lint all code. EVERY feature: the ACP lane (joy-ai `acp`) is only
# compiled behind its feature, and without this flag it was never linted
# here at all - a missing dependency and a duplicated attribute in it
# surfaced from the desktop build instead (JOY-0280-A5, 2026-09-09).
lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# The Windows-only code (joy-process: CREATE_NO_WINDOW, GetConsoleWindow)
# never compiles on a Linux or macOS box, so a typo there used to surface
# on the windows-latest CI leg only. Type-checking it for the Windows
# target needs no linker; the target's std is a one-time download.
# Type-check the Windows-only code from any host
check-windows:
    rustup target add x86_64-pc-windows-msvc >/dev/null
    cargo check -p joy-process --all-targets --target x86_64-pc-windows-msvc

# Abort if the local Rust stable toolchain is behind the latest release.
# Prevents clippy-version drift between local and CI.
[private]
_toolchain-check:
    #!/usr/bin/env bash
    if rustup check stable 2>/dev/null | grep -q "Update available"; then
        echo "Error: Rust stable toolchain is outdated. Run 'rustup update stable'."
        exit 1
    fi

# Refresh the in-crate copy of the Tutorial used by `joy tutorial`.
# The canonical doc is docs/user/Tutorial.md; cargo package needs a
# copy inside the joy-cli crate. See JOY-017F-FD.
sync-tutorial:
    mkdir -p crates/joy-cli/docs/user crates/joy-cli/docs/ai
    cp docs/user/Tutorial.md crates/joy-cli/docs/user/Tutorial.md
    cp docs/ai/Tutorial.md crates/joy-cli/docs/ai/Tutorial.md

# Run fmt-check, lint, test
# The fast gate, for every commit: static checks plus the functional
# core. Seconds, not minutes, so nobody is tempted to skip it.
check: _toolchain-check sync-tutorial fmt-check lint check-windows guard-vcs guard-certificate-check test-unit test-cmd test-smoke

# Git lives in ONE place (JOY-0265-D7): joy-core/src/vcs, plus the chat
# store's object plumbing (a git-object database, its own storage layer).
# git2 elsewhere is compile-guarded (dev-dependency only); this guards
# the git BINARY calls. Tests may shell git to build fixtures.
guard-vcs:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    bad=0
    for f in $(grep -rl 'command("git")' crates/*/src --include='*.rs' | grep -v 'crates/joy-core/src/vcs/' | grep -v 'crates/joy-process/'); do
        test_start=$(grep -n '#\[cfg(test)\]' "$f" | head -1 | cut -d: -f1)
        for line in $(grep -n 'command("git")' "$f" | cut -d: -f1); do
            if [ -z "$test_start" ] || [ "$line" -lt "$test_start" ]; then
                echo "guard-vcs: $f calls git directly (line $line); git belongs in joy-core/src/vcs"
                bad=1
            fi
        done
    done
    exit $bad

# The ONE `certificate_check` closure (design D1.4a). git2 0.21 holds
# exactly one such slot per contact (remote_callbacks.rs:27), so a
# second installer anywhere in the tree would silently take the ssh
# host key decision away from joy_core::vcs::certificates and hand it
# back to libgit2, which reads one file with strcmp. The module that
# OWNS the closure may name it; exactly one call site may install it.
guard-certificate-check:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    owner='crates/joy-core/src/vcs/certificates'
    installer='crates/joy-core/src/vcs/forge.rs'
    # EVERY Rust file of the workspace, not only crates/*/src: a
    # builder in an integration test or in a build script takes the
    # decision away from joy just as quietly as one beside the engine.
    # Both spellings of the builder count, `new()` and `default()`.
    builder='RemoteCallbacks::(new|default)\('
    bad=0
    while IFS=: read -r file line _; do
        case "$file" in
            "$owner".rs|"$owner"/*) continue ;;
            "$installer") continue ;;
        esac
        echo "guard-certificate-check: $file:$line installs a second certificate_check; the one closure lives in $owner.rs"
        bad=1
    done < <(grep -rn --include='*.rs' 'certificate_check(' crates || true)
    # occurrences, not lines: two installs written on one line are two
    installs=$(grep -o 'certificate_check(' "$installer" | wc -l | tr -d '[:space:]')
    if [ "$installs" != "1" ]; then
        echo "guard-certificate-check: $installer installs certificate_check $installs times, expected exactly 1"
        bad=1
    fi
    # and the other half of the same rule: the closure goes into EVERY
    # RemoteCallbacks joy builds, which holds only while they are all
    # built in the one place that installs it
    while IFS=: read -r file line _; do
        case "$file" in
            "$owner".rs|"$owner"/*) continue ;;
            "$installer") continue ;;
        esac
        echo "guard-certificate-check: $file:$line builds a RemoteCallbacks outside $installer, so it carries no certificate_check"
        bad=1
    done < <(grep -rnE --include='*.rs' "$builder" crates || true)
    builds=$(grep -oE "$builder" "$installer" | wc -l | tr -d '[:space:]')
    if [ "$builds" != "1" ]; then
        echo "guard-certificate-check: $installer builds RemoteCallbacks $builds times, expected exactly 1"
        bad=1
    fi
    exit $bad

# Take the pinned host keys off the three public forges again and check
# them against crates/joy-core/data/host-keys.published.json and
# against the fingerprints the forges publish (design D1.4a: the
# Codeberg pin is a blob taken once and checked against a page that
# publishes fingerprints only). This is the build step of D1.4a, and CI
# runs it every night (.github/workflows/ci.yaml, job host-key-pins),
# so a rotation or a hand-edited pin is noticed by a job and not by a
# person whose contact failed. Needs the network; the offline half of
# the same check is the unit test pins_match_the_published_fingerprints.
# The file it checks is the one parked BESIDE the release: the pin file
# a release ships is empty while decision 23 is open.
check-host-key-pins:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    pins=crates/joy-core/data/host-keys.published.json
    bad=0
    # 1. every recorded blob is the key its recorded fingerprint names
    while read -r host type key fingerprint; do
        computed=$(printf '%s %s\n' "$type" "$key" | ssh-keygen -lf - | awk '{print $2}')
        if [ "$computed" != "$fingerprint" ]; then
            echo "pin $host $type: the blob hashes to $computed, the file records $fingerprint"
            bad=1
        fi
    done < <(jq -r '.hosts[] | .host as $h | .keys[] | "\($h) \(.type) \(.key) \(.fingerprint)"' "$pins")
    # 2. the host still serves exactly the recorded blobs
    while read -r host port; do
        scanned=$(ssh-keyscan -T 20 -p "$port" -t rsa,ecdsa,ed25519 "$host" 2>/dev/null \
            | grep -v '^#' | awk '{print $2" "$3}' | sort)
        recorded=$(jq -r --arg h "$host" '.hosts[] | select(.host==$h) | .keys[] | "\(.type) \(.key)"' "$pins" | sort)
        if [ "$scanned" != "$recorded" ]; then
            echo "pin $host:$port: the keys the host serves are not the recorded ones"
            diff <(echo "$recorded") <(echo "$scanned") || true
            bad=1
        fi
    done < <(jq -r '.hosts[] | "\(.host) \(.port)"' "$pins")
    # 3. the page each forge publishes still carries the recorded
    # fingerprints. GitHub publishes them as JSON, GitLab and Codeberg
    # as prose, so the prose pages are searched for the strings.
    github=$(curl -sS --max-time 30 https://api.github.com/meta | jq -r '.ssh_key_fingerprints[]')
    pages=$(curl -sS --max-time 30 https://docs.gitlab.com/user/gitlab_com/ \
            https://docs.codeberg.org/security/ssh-fingerprint/)
    while read -r host fingerprint; do
        bare=${fingerprint#SHA256:}
        case "$host" in
            # a here string and not a pipe: `grep -q` leaves the
            # writer with SIGPIPE, and under `pipefail` that reads as a
            # failed search for a fingerprint that was found
            github.com|ssh.github.com)
                grep -qxF "$bare" <<<"$github" || { echo "pin $host: $fingerprint is not on GitHub's page"; bad=1; } ;;
            *)
                grep -qF "$bare" <<<"$pages" || { echo "pin $host: $fingerprint is not on the forge's page"; bad=1; } ;;
        esac
    done < <(jq -r '.hosts[] | .host as $h | .keys[] | "\($h) \(.fingerprint)"' "$pins")
    if [ "$bad" = "0" ]; then echo "host key pins: all blobs, hosts and published fingerprints agree"; fi
    exit $bad

# Everything, for the nightly run and before a release.
check-all: _toolchain-check sync-tutorial fmt-check lint test

# Lint commit messages for Joy item references (default: main..HEAD)
lint-commits base="main":
    #!/usr/bin/env bash
    set -euo pipefail
    ROOT=$(git rev-parse --show-toplevel)
    ACRONYM=$(grep -E '^acronym:' "$ROOT/.joy/project.yaml" 2>/dev/null | head -1 | sed "s/^acronym:[[:space:]]*//" | tr -d "\"'")
    if [ -z "$ACRONYM" ]; then
        echo "error: no project acronym found in .joy/project.yaml" >&2
        exit 1
    fi
    PATTERN="${ACRONYM}-[0-9A-Fa-f]{4}"
    RANGE="{{base}}..HEAD"
    if ! git rev-parse "{{base}}" >/dev/null 2>&1; then
        echo "error: base ref '{{base}}' not found" >&2
        exit 1
    fi
    COMMITS=$(git log --format="%H %s" "$RANGE" 2>/dev/null)
    if [ -z "$COMMITS" ]; then
        echo "No commits to check in $RANGE"
        exit 0
    fi
    FAILED=0
    while IFS= read -r line; do
        HASH="${line%% *}"
        MSG="${line#* }"
        SHORT="${HASH:0:8}"
        if echo "$MSG" | grep -qE "$PATTERN"; then
            continue
        fi
        if echo "$MSG" | grep -qF '[no-item]'; then
            continue
        fi
        echo "  $SHORT $MSG" >&2
        FAILED=$((FAILED + 1))
    done <<< "$COMMITS"
    if [ "$FAILED" -gt 0 ]; then
        echo "" >&2
        echo "error: $FAILED commit(s) missing $ACRONYM-XXXX item reference" >&2
        echo "  = help: add an item ID or [no-item] tag to commit messages" >&2
        exit 1
    fi
    echo "All commits reference a Joy item."

# Check tools and deps
doctor:
    #!/usr/bin/env bash
    red=$'\033[31m' orange=$'\033[38;5;208m' reset=$'\033[0m'
    ok()   { local v; v=$("$1" --version 2>/dev/null) && echo "  $2: $v" || echo "  $2: ok"; }
    miss() { printf "  %s%s: MISSING%s\n" "$red" "$1" "$reset"; }
    opt()  { printf "  %s%s: MISSING (optional, %s)%s\n" "$orange" "$1" "$2" "$reset"; }
    command -v cargo         >/dev/null && ok cargo cargo           || miss cargo
    command -v rustfmt       >/dev/null && ok rustfmt rustfmt       || miss rustfmt
    command -v clippy-driver >/dev/null && ok clippy-driver clippy  || miss clippy
    command -v git           >/dev/null && ok git git               || miss git
    cargo --list 2>/dev/null | grep -q insta    && echo "  cargo-insta: ok"    || miss "cargo-insta"
    cargo --list 2>/dev/null | grep -q 'llvm-cov' && echo "  cargo-llvm-cov: ok" || opt "cargo-llvm-cov" "cargo install cargo-llvm-cov"
    cargo --list 2>/dev/null | grep -q watch    && echo "  cargo-watch: ok"    || opt "cargo-watch" "cargo install cargo-watch"
    command -v bats          >/dev/null && ok bats bats             || miss "bats (pacman -S bats)"
    command -v jq            >/dev/null && ok jq jq                 || miss "jq (pacman -S jq)"
    command -v gh            >/dev/null && ok gh "gh (GitHub CLI)" || opt "gh" "https://cli.github.com"

# Install cargo tools for development
setup:
    cargo install cargo-insta

# Install to ~/.local/bin/ (joy plus the plugins: joy-bi, the connector
# joy-forge, and the legacy forge plugins joy-core's identity fallback
# queries, JOY-0251-AA)
#
# The layout an install has (JOY-029E-9D): joy and joy-forge in the bin
# directory, so joy resolves the connector by the name order of D2.2,
# and the host key pin file of design D1.4a under the prefix's share
# directory, which is the second candidate of pins::candidates and the
# path the installers of W1 write. Not into ~/.local/bin: a pin file
# there is the first candidate and would shadow the share copy for
# good. The `rm` line clears exactly that, because an earlier revision
# of this recipe wrote one. The three legacy names stay in the recipe
# on purpose: a machine that carries them is the shadowing case of D2.2a.
install:
    cargo build --release -p joy-cli -p joy-bi -p joy-github -p joy-gitlab -p joy-gitea && mkdir -p ~/.local/bin ~/.local/share/joy && cp target/release/joy target/release/joy-forge target/release/joy-bi target/release/joy-github target/release/joy-gitlab target/release/joy-gitea ~/.local/bin/ && rm -f ~/.local/bin/host-keys.json && cp crates/joy-core/data/host-keys.json ~/.local/share/joy/

# Auto-commit known generated files (.joy/, lockfiles)
[private]
auto-commit:
    #!/usr/bin/env bash
    files=(.joy/ Cargo.lock package-lock.json yarn.lock)
    staged=false
    for f in "${files[@]}"; do
        if git status --porcelain "$f" 2>/dev/null | grep -q .; then
            git add "$f"
            staged=true
        fi
    done
    if [ "$staged" = true ]; then
        git commit --quiet -m "chore: update generated files [no-item]"
        echo "Committed pending changes."
    fi

# Local-only release: bump version files, refresh Cargo.lock, record,
# commit, tag. No push, no crates.io publish, no forge release.
# Follow with `just publish` once this succeeds.
# Release (bump: patch, minor, or major)
release bump="patch":
    #!/usr/bin/env bash
    set -euo pipefail
    if git describe --tags --exact-match HEAD >/dev/null 2>&1; then
        echo "No changes since last tag, skipping."
        exit 0
    fi
    just auto-commit
    if ! command -v joy >/dev/null 2>&1 || ! [ -f ".joy/project.yaml" ]; then
        echo "No Joy project found. Use joy init to set up."
        exit 1
    fi
    if ! joy release show >/dev/null 2>&1; then
        echo "No items closed since last release."
        exit 0
    fi
    # Joy logs every invocation to .joy/logs/YYYY-MM-DD.log, so the
    # `joy release show` above may have dirtied the tree. Absorb those
    # writes before the clean check, otherwise the release aborts on
    # its own audit trail.
    just auto-commit
    if [ -n "$(git status --porcelain)" ]; then
        echo "Error: working tree is not clean."
        exit 1
    fi
    echo "Updating external dependencies..."
    cargo update
    just auto-commit
    echo "Bumping version files..."
    joy release bump "{{bump}}"
    echo "Refreshing Cargo.lock..."
    cargo update --workspace
    echo "Checking (format, lint, test)..."
    if ! just check > /dev/null 2>&1; then
        echo "Checks failed. Run 'just check' for details. Rolling bump back."
        git restore crates/ Cargo.lock
        exit 1
    fi
    joy release record "{{bump}}"
    tag=$(git describe --tags --exact-match HEAD 2>/dev/null || echo "unknown")
    echo "Tagged ${tag} locally. Run 'just publish' to ship."

# Publish workspace crates to crates.io, then push and create the
# forge release. Reads CARGO_REGISTRY_TOKEN from the environment
# (umbrella's `.env` is loaded automatically; CI sets it from its
# secret store). Skips crates whose current version is already
# published, so re-running after a partial failure is safe. See
# ADR-032 for the local-first release paradigm.
# Upload crates to crates.io only. Idempotent: already-uploaded
# versions are skipped. CI's publish.yml calls this directly; the
# forge release is handled separately by `joy release publish`.
publish-crates: sync-tutorial
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -z "${CARGO_REGISTRY_TOKEN:-}" ]; then
        echo "Error: CARGO_REGISTRY_TOKEN is not set."
        echo "  - Local: add it to the umbrella's .env (see .env.example)."
        echo "  - CI: export it from the runner's secret store."
        exit 1
    fi
    # Order matters: dependents after dependencies.
    # joy-forge-net rides BEFORE joy-core, because the engine takes the
    # one NO_PROXY matcher from it (JOY-02A3-E4); joy-bi rides after
    # joy-core (its only internal dependency); the three forge
    # connectors ride after joy-forge-net, which is what they share,
    # and joy-cli after all of them, because it links the three and
    # ships the `joy-forge` binary (JOY-0298-E4). Every workspace
    # member is either in this list or carries publish = false -- the
    # gap that made JOY-0247-E1. `publish_order_matches_the_crates`
    # (joy-core/tests/publish_order.rs) holds this list to its own rule,
    # so a new edge cannot break a release halfway through again.
    crates=(joy-process joy-model joy-chat joy-forge-net joy-core joy-bi joy-github joy-gitlab joy-gitea joy-chat-store joy-ai joy-cli)
    for crate in "${crates[@]}"; do
        version=$(cargo pkgid --quiet -p "$crate" 2>/dev/null | sed 's/.*[#@]\(.*\)/\1/')
        if [ -z "$version" ]; then
            echo "Warning: could not resolve version for $crate, skipping."
            continue
        fi
        # Idempotency hint via cargo search (lags behind the registry by a
        # minute or two, so we still need the post-publish guard below).
        if cargo search "$crate" --limit 1 2>/dev/null | grep -qE "^$crate = \"$version\""; then
            echo "$crate $version already on crates.io, skipping."
            continue
        fi
        echo "Publishing $crate $version..."
        # Capture output; treat "already uploaded" as success so a duplicate
        # run (e.g. local + CI on the same tag) is harmless instead of
        # erroring out the whole publish step.
        if ! out=$(cargo publish -p "$crate" 2>&1); then
            # Two cargo error variants both mean "version already published":
            # - "is already uploaded": registry rejected the upload
            # - "already exists on crates.io index": cargo's pre-check
            if echo "$out" | grep -qE "is already uploaded|already exists on crates.io index"; then
                echo "$crate $version already on crates.io (registry confirmed), skipping."
            else
                echo "$out" >&2
                exit 1
            fi
        else
            echo "$out"
        fi
        # Brief wait so the next crate (which may depend on this one) sees
        # the new version in the registry index.
        sleep 5
    done
    echo "crates.io uploads complete."

# Full publish: upload crates + push + create forge release. This is
# what `just release-all -P` in the umbrella calls per sub. Running
# crates.io upload first means a failed upload leaves only a local
# tag to drop.
# Publish workspace crates, then push + forge release.
publish: publish-crates
    joy release publish
