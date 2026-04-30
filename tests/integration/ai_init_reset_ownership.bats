#!/usr/bin/env bats
# JOY-00D1-3C: joy ai init must not skip configuration just because a
# user-shared instruction file already exists, and joy ai reset must
# not delete user-owned files in shared tool directories.

load setup

setup_fake_ai_tools() {
    BIN_DIR="$TEST_DIR/fake-bin"
    mkdir -p "$BIN_DIR"
    for cmd in claude qwen vibe gh; do
        printf '#!/bin/sh\nexit 0\n' > "$BIN_DIR/$cmd"
        chmod +x "$BIN_DIR/$cmd"
    done
    PATH="$BIN_DIR:$PATH"
}

# --- Detection: pre-existing shared instruction file does not block setup ---

@test "joy ai init configures Claude even when .claude/CLAUDE.md already exists" {
    setup_human_auth
    setup_fake_ai_tools

    mkdir -p .claude
    echo "# user-managed CLAUDE.md, no joy block" > .claude/CLAUDE.md

    joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null 2>/dev/null

    [ -f .claude/skills/joy/SKILL.md ]
    grep -q "<!-- joy:start -->" .claude/CLAUDE.md
}

@test "joy ai init configures Qwen even when .qwen/QWEN.md already exists" {
    setup_human_auth
    setup_fake_ai_tools

    mkdir -p .qwen
    echo "# user-managed QWEN.md, no joy block" > .qwen/QWEN.md

    joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null 2>/dev/null

    [ -f .qwen/skills/joy/SKILL.md ]
    grep -q "<!-- joy:start -->" .qwen/QWEN.md
}

@test "joy ai init configures Copilot even when copilot-instructions.md already exists" {
    setup_human_auth
    setup_fake_ai_tools

    mkdir -p .github
    echo "# user-managed copilot-instructions, no joy block" > .github/copilot-instructions.md

    joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null 2>/dev/null

    [ -d .github/agents ]
    grep -q "<!-- joy:start -->" .github/copilot-instructions.md
}

# --- Reset: user-owned files in shared dirs survive ---

@test "joy ai reset --tool claude preserves user-owned files in .claude/" {
    setup_human_auth
    setup_fake_ai_tools
    joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null 2>/dev/null

    # User-owned file in .claude/ that joy did not create.
    echo "user keybindings" > .claude/my-keybindings.json

    joy ai reset --tool claude --force 2>/dev/null

    [ -f .claude/my-keybindings.json ]
    [ ! -d .claude/skills/joy ]
}

@test "joy ai reset --tool qwen preserves user-owned files in .qwen/" {
    setup_human_auth
    setup_fake_ai_tools
    joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null 2>/dev/null

    echo "notes" > .qwen/my-notes.txt

    joy ai reset --tool qwen --force 2>/dev/null

    [ -f .qwen/my-notes.txt ]
    [ ! -d .qwen/skills/joy ]
}

@test "joy ai reset --tool vibe preserves user-owned files in .vibe/" {
    setup_human_auth
    setup_fake_ai_tools
    joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null 2>/dev/null

    mkdir -p .vibe
    echo "user data" > .vibe/data.json

    joy ai reset --tool vibe --force 2>/dev/null

    [ -f .vibe/data.json ]
    [ ! -d .vibe/skills/joy ]
}

@test "joy ai reset strips joy-block from CLAUDE.md but keeps user content" {
    setup_human_auth
    setup_fake_ai_tools
    mkdir -p .claude
    cat > .claude/CLAUDE.md <<'EOF'
# Custom user content above

User-specific instructions here.
EOF

    joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null 2>/dev/null
    grep -q "<!-- joy:start -->" .claude/CLAUDE.md

    joy ai reset --tool claude --force 2>/dev/null

    [ -f .claude/CLAUDE.md ]
    grep -q "User-specific instructions here" .claude/CLAUDE.md
    ! grep -q "<!-- joy:start -->" .claude/CLAUDE.md
}
