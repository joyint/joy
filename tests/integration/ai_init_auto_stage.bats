#!/usr/bin/env bats
# joy ai init auto-stages every file it writes (JOY-0184-4A).

load setup

@test "joy init + joy ai init leave no joy-managed file unstaged" {
    joy init --name "Test Project" 2>/dev/null
    joy auth init --passphrase "$TEST_PASSPHRASE" >/dev/null
    joy ai init --passphrase "$TEST_PASSPHRASE" </dev/null 2>/dev/null

    # Every file joy created or modified must be in the staged set.
    # Joy-managed paths to check:
    #   .joy/project.yaml, .joy/config.defaults.yaml
    #   .gitignore, .gitattributes
    #   SECURITY.md, CONTRIBUTING.md
    #   VISION.md, ARCHITECTURE.md
    #   At least one tool config dir (.claude/, .qwen/, .vibe/, AGENTS.md,
    #     .github/copilot-instructions.md) depending on what was detected.
    local staged
    staged=$(git diff --name-only --cached)

    for f in .joy/project.yaml .joy/config.defaults.yaml .gitignore .gitattributes; do
        if [ -e "$f" ]; then
            echo "$staged" | grep -qE "^${f}$" || { echo "missing from staged: $f"; return 1; }
        fi
    done
    # Optional but joy-managed when present.
    for f in SECURITY.md CONTRIBUTING.md VISION.md ARCHITECTURE.md; do
        if [ -e "$f" ]; then
            echo "$staged" | grep -qE "^${f}$" || { echo "missing from staged: $f"; return 1; }
        fi
    done

    # No joy-managed file should be in `git ls-files --others`.
    local untracked
    untracked=$(git ls-files --others --exclude-standard)
    local leak=""
    for f in $untracked; do
        case "$f" in
            .joy/*|.claude/*|.qwen/*|.vibe/*|.github/*|AGENTS.md|SECURITY.md|CONTRIBUTING.md|docs/dev/*)
                leak="$leak $f"
                ;;
        esac
    done
    [ -z "$leak" ] || { echo "untracked joy-managed files:$leak"; return 1; }
}
