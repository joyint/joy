#!/usr/bin/env bats
# Shared AI instructions use the authenticated member, never the tool's name.

load setup

@test "reinitializing Vibe replaces a stale identity in shared instructions" {
    setup_human_auth
    joy ai init --tool vibe --passphrase "$TEST_PASSPHRASE" </dev/null

    grep -q 'data.member' AGENTS.md
    grep -q 'data.session_env' AGENTS.md
    grep -q 'data.delegated_by' AGENTS.md
    ! grep -q 'vibe' AGENTS.md
    ! grep -q 'Co-Authored-By: Mistral' AGENTS.md
    ! grep -q 'Your interaction level:' AGENTS.md

    sed -i 's/data.member/vibe/' AGENTS.md
    joy ai init --tool vibe </dev/null
    grep -q 'data.member' AGENTS.md
    ! grep -q 'vibe' AGENTS.md
}
