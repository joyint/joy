#!/usr/bin/env bats
# GitHub ships ONE Copilot CLI under two commands, and `joy ai init` must
# DETECT it under either: `copilot` from npm, or `gh copilot` through the
# GitHub CLI. What it must NOT count is a bare `gh` — that binary sits on
# virtually every CI runner and dev machine and says nothing about whether
# Copilot is installed behind it.
#
# Every test here drives AUTO-DETECTION (no `--tool`, which deliberately
# bypasses it) and runs on a PATH holding only the shims it asks for, so a
# real copilot or gh on the developer's machine cannot decide the outcome.
# The gh shim mimics the real one where detection turns on it: it answers
# `copilot -- --version` only when Copilot sits behind it.

load setup

# PATH = the shims, the joy binary, and the system basics joy shells out
# to (`which`, git). Nothing else.
isolated_path() {
    BIN_DIR="$TEST_DIR/fake-bin"
    mkdir -p "$BIN_DIR"
    PATH="$BIN_DIR:$(dirname "$(command -v joy)"):/usr/bin:/bin"
    # no tool of any kind unless a test installs one
    run which copilot
    [ "$status" -ne 0 ]
}

fake_copilot() {
    printf '#!/bin/sh\necho "GitHub Copilot CLI 1.0.85."\n' > "$BIN_DIR/copilot"
    chmod +x "$BIN_DIR/copilot"
}

# $1 = with-copilot: Copilot answers behind gh. Otherwise gh is there and
# Copilot is not, which is the trap this whole file exists for.
fake_gh() {
    if [ "$1" = "with-copilot" ]; then
        cat > "$BIN_DIR/gh" <<'SH'
#!/bin/sh
[ "$1" = "copilot" ] && { echo "GitHub Copilot CLI 1.0.85."; exit 0; }
exit 1
SH
    else
        cat > "$BIN_DIR/gh" <<'SH'
#!/bin/sh
# what the real gh says without a terminal and without Copilot installed
[ "$1" = "copilot" ] && { echo "Copilot CLI not installed" >&2; exit 1; }
exit 1
SH
    fi
    chmod +x "$BIN_DIR/gh"
}

# </dev/null accepts every prompt's default, as in ai.bats.
#
# The editor signal is always stated, never inherited: run this suite
# inside a real VS Code terminal and an inherited TERM_PROGRAM would make
# every "no Copilot here" test detect one. $EDITOR_SIGNAL is what each
# test decides.
init_and_detect() {
    TERM_PROGRAM="${EDITOR_SIGNAL:-none}" joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null 2>/dev/null || true
}

@test "ai init detects Copilot under the plain copilot command" {
    setup_human_auth
    isolated_path
    fake_copilot

    init_and_detect

    [ -f .github/copilot-instructions.md ]
    grep -q "ai:copilot@joy" .joy/project.yaml
}

@test "ai init detects Copilot under gh copilot alone" {
    setup_human_auth
    isolated_path
    fake_gh with-copilot
    [ ! -e "$BIN_DIR/copilot" ]

    init_and_detect

    [ -f .github/copilot-instructions.md ]
    grep -q "ai:copilot@joy" .joy/project.yaml
}

@test "ai init detects Copilot when both commands are installed" {
    setup_human_auth
    isolated_path
    fake_copilot
    fake_gh with-copilot

    init_and_detect

    [ -f .github/copilot-instructions.md ]
    grep -q "ai:copilot@joy" .joy/project.yaml
}

@test "a bare gh is not Copilot and registers no member" {
    setup_human_auth
    isolated_path
    fake_gh without-copilot

    init_and_detect

    [ ! -f .github/copilot-instructions.md ]
    ! grep -q "ai:copilot@joy" .joy/project.yaml
}

# Copilot is not only a CLI. An editor of the VS Code family has Copilot
# Chat built in and reads the very files this tool writes, so somebody who
# never installed a binary is still a Copilot user. They used to be told
# to register a member by hand; now `joy ai init` offers it where the
# instructions actually apply.
@test "ai init offers Copilot in a VS Code family terminal with no CLI at all" {
    setup_human_auth
    isolated_path
    [ ! -e "$BIN_DIR/copilot" ]
    [ ! -e "$BIN_DIR/gh" ]

    EDITOR_SIGNAL=vscode init_and_detect

    [ -f .github/copilot-instructions.md ]
    grep -q "ai:copilot@joy" .joy/project.yaml
}

# The same run outside such an editor finds nothing: the signal is what
# makes the difference, not a tool that was there all along.
@test "no editor and no CLI means no Copilot" {
    setup_human_auth
    isolated_path

    EDITOR_SIGNAL=tmux init_and_detect

    [ ! -f .github/copilot-instructions.md ]
    ! grep -q "ai:copilot@joy" .joy/project.yaml
}

# The probe must not be mistaken for consent: the real `gh` downloads the
# Copilot CLI unasked, about 166 MB, when it believes it runs in CI. Joy
# is asking a question there, not accepting an offer, so it strips those
# variables first. This shim records it if any of them survives.
@test "the gh probe never runs in CI mode" {
    setup_human_auth
    isolated_path
    export PROBE_MARKER="$TEST_DIR/probe-ran-unattended"
    cat > "$BIN_DIR/gh" <<'SH'
#!/bin/sh
[ "$1" = "copilot" ] || exit 1
if [ -n "${CI:-}" ] || [ -n "${BUILD_NUMBER:-}" ] || [ -n "${RUN_ID:-}" ]; then
    # the real gh would start a silent download here
    echo "unattended" > "$PROBE_MARKER"
    exit 1
fi
echo "GitHub Copilot CLI 1.0.85."
SH
    chmod +x "$BIN_DIR/gh"

    export CI=1 BUILD_NUMBER=42 RUN_ID=7
    init_and_detect

    [ ! -f "$PROBE_MARKER" ]
    # and the honest answer survives the stripping: Copilot IS here
    grep -q "ai:copilot@joy" .joy/project.yaml
}
