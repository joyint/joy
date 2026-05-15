#!/usr/bin/env bats
# joy ai init and joy update must never stage paths that are
# listed in the joy-managed .gitignore block. Regression guard
# for the bug where every AI-tool file (.claude/, .vibe/,
# .github/copilot-*, AGENTS.md, ...) silently ended up in the
# index on first run, then `joy update` flooded the operator with
# "auto-git add failed: paths are ignored" warnings.

load setup

# Helper: every path matching one of the AI-tool gitignore globs.
# Mirrors the joy-managed gitignore block; if joy ever ships new
# AI-tool patterns, extend this list too.
_ai_tool_staged() {
    git diff --name-only --cached | grep -E '^(\.claude/|\.qwen/|\.vibe/|\.github/copilot|\.github/copilot-instructions\.md|\.github/prompts/|\.github/agents/|AGENTS\.md)' || true
}

@test "joy ai init does not stage gitignored AI-tool files" {
    setup_human_auth
    joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null >/dev/null 2>&1

    local leaked
    leaked=$(_ai_tool_staged)
    [ -z "$leaked" ] || {
        echo "AI-tool files were staged despite being gitignored:"
        printf '  %s\n' $leaked
        return 1
    }
}

@test "joy update refreshes a stale AI-tool file without warnings or staging" {
    setup_human_auth
    joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null >/dev/null 2>&1

    # Forge a stale rendition of a rendered AI-tool file by
    # overwriting it with content that won't match the current
    # template. Pick SKILL.md since joy ai init writes one for
    # every configured tool.
    local skill_path
    skill_path=$(find .claude/skills .vibe/skills -name SKILL.md 2>/dev/null | head -1)
    [ -n "$skill_path" ] || skip "no AI tool with SKILL.md was configured"
    echo "STALE-MARKER-$(date +%s%N)" > "$skill_path"

    # Sanity: joy update --check must report the project as stale
    # before the refresh, otherwise the test would silently pass on
    # a no-op run. `joy update --check` exits non-zero when stale.
    run joy update --check
    [ "$status" -ne 0 ]
    [[ "$output" == *"outdated"* ]] || [[ "$output" == *"Stale items"* ]]

    run joy update
    [ "$status" -eq 0 ]
    # The forged file must have been rewritten back to the template.
    ! grep -q "^STALE-MARKER-" "$skill_path"
    # No "paths are ignored" wall of warnings.
    [[ "$output" != *"auto-git add failed"* ]]
    [[ "$output" != *"paths are ignored"* ]]
    # And nothing under the AI-tool gitignore patterns ended up
    # staged as a side effect.
    local leaked
    leaked=$(_ai_tool_staged)
    [ -z "$leaked" ] || {
        echo "joy update staged ignored AI-tool files:"
        printf '  %s\n' $leaked
        return 1
    }
}

@test "joy update does not stage gitignored AI-tool files" {
    setup_human_auth
    joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null >/dev/null 2>&1

    # Capture the staged set right after ai init so we can detect
    # anything new that joy update would slip in.
    local before
    before=$(git diff --name-only --cached | sort)

    # Mark every rendered AI-tool file as stale so joy update has
    # to rewrite each one.
    find .claude .vibe .github -type f 2>/dev/null \
        | while read -r f; do echo "" >> "$f"; done
    [ -f AGENTS.md ] && echo "" >> AGENTS.md

    run joy update
    [ "$status" -eq 0 ]
    [[ "$output" != *"auto-git add failed"* ]]
    [[ "$output" != *"paths are ignored"* ]]

    # No AI-tool path may end up staged.
    local leaked
    leaked=$(_ai_tool_staged)
    [ -z "$leaked" ] || {
        echo "joy update staged ignored AI-tool files:"
        printf '  %s\n' $leaked
        return 1
    }

    # And the staged set must not have gained any new AI-tool entry
    # beyond the (already-empty) AI-tool subset of `before`.
    local after
    after=$(git diff --name-only --cached | sort)
    [ "$before" = "$after" ] || {
        echo "joy update changed the staged set:"
        diff <(echo "$before") <(echo "$after") || true
        return 1
    }
}
