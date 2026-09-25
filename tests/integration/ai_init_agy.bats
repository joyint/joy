#!/usr/bin/env bats
# Antigravity uses the shared project instructions and its own skill and agents.

load setup

@test "ai init detects agy and generates discoverable Joy artifacts" {
    setup_human_auth
    mkdir -p "$TEST_DIR/fake-bin"
    printf '#!/bin/sh\nexit 0\n' > "$TEST_DIR/fake-bin/agy"
    chmod +x "$TEST_DIR/fake-bin/agy"
    export PATH="$TEST_DIR/fake-bin:$PATH"

    joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null

    grep -q 'ai:agy@joy' .joy/project.yaml
    grep -q 'data.member' AGENTS.md
    ! grep -q 'ai:agy@joy' AGENTS.md
    [ -f .agents/skills/joy/SKILL.md ]
    grep -q '^name: joy$' .agents/skills/joy/SKILL.md
    [ -f .agents/skills/joy/setup.md ]
    for agent in conceiver planner designer implementer tester reviewer documenter; do
        [ -f ".agents/agents/joy-$agent/agent.md" ]
        grep -q "^name: joy-$agent$" ".agents/agents/joy-$agent/agent.md"
    done
}

@test "ai init --tool agy works without the launcher and refreshes stale files" {
    setup_human_auth
    joy ai init --tool agy --passphrase "$TEST_PASSPHRASE" </dev/null

    echo 'obsolete' > .agents/agents/joy-reviewer/agent.md
    echo 'obsolete' > .agents/skills/joy/setup.md
    joy ai init --tool agy </dev/null
    grep -q '^name: joy-reviewer$' .agents/agents/joy-reviewer/agent.md
    grep -q 'First session' .agents/skills/joy/setup.md

    echo 'obsolete' > .agents/skills/joy/SKILL.md
    run joy update --check
    [ "$status" -ne 0 ]
    [[ "$output" == *"Google Antigravity"* ]]
    joy update >/dev/null
    grep -q '^name: joy$' .agents/skills/joy/SKILL.md

    touch -t 202001010000 .agents/skills/joy/SKILL.md
    before=$(stat -c %Y .agents/skills/joy/SKILL.md)
    joy ai init --tool agy </dev/null
    [ "$(stat -c %Y .agents/skills/joy/SKILL.md)" = "$before" ]
}

@test "resetting Vibe and Antigravity separately preserves their shared instructions" {
    setup_human_auth
    echo '# User instructions' > AGENTS.md
    joy ai init --tool vibe --passphrase "$TEST_PASSPHRASE" </dev/null
    joy ai init --tool agy --passphrase "$TEST_PASSPHRASE" </dev/null
    mkdir -p .agents/agents/custom .agents/agents/joy-reviewer
    echo 'user agent' > .agents/agents/custom/agent.md
    echo 'user note' > .agents/agents/joy-reviewer/note.md
    echo 'user skill note' > .agents/skills/joy/note.md

    joy ai reset --tool vibe --force
    grep -q '<!-- joy:start -->' AGENTS.md
    [ -f .agents/skills/joy/SKILL.md ]
    joy ai reset --tool agy --force
    grep -q '# User instructions' AGENTS.md
    ! grep -q '<!-- joy:start -->' AGENTS.md
    [ ! -f .agents/skills/joy/SKILL.md ]
    [ ! -f .agents/agents/joy-reviewer/agent.md ]
    [ -f .agents/agents/joy-reviewer/note.md ]
    [ -f .agents/agents/custom/agent.md ]
    [ -f .agents/skills/joy/note.md ]
}

@test "resetting Antigravity before Vibe keeps the shared Joy block until last reader" {
    setup_human_auth
    joy ai init --tool vibe --passphrase "$TEST_PASSPHRASE" </dev/null
    joy ai init --tool agy --passphrase "$TEST_PASSPHRASE" </dev/null

    joy ai reset --tool agy --force
    grep -q '<!-- joy:start -->' AGENTS.md
    [ -f .vibe/skills/joy/SKILL.md ]
    joy ai reset --tool vibe --force
    [ ! -f AGENTS.md ]
}
