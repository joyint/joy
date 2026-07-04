#!/usr/bin/env bats
# joy ai init --tool sets up exactly one tool (JOY-01F9 settings toggle).

load setup

@test "ai init --tool claude registers only claude" {
    joy init --name "Tool Test" 2>/dev/null
    joy auth init --passphrase "correct horse battery staple" >/dev/null
    run joy ai init --tool claude --passphrase "correct horse battery staple"
    [ "$status" -eq 0 ]
    grep -q "ai:claude@joy" .joy/project.yaml
    ! grep -q "ai:qwen@joy" .joy/project.yaml
    [ -f .claude/CLAUDE.md ]
    [ ! -d .qwen ]

    # reset --tool takes it away again
    run joy ai reset --tool claude --force
    [ "$status" -eq 0 ]
    [ ! -f .claude/CLAUDE.md ]

    # unknown tool errors clearly
    run joy ai init --tool nope --passphrase "correct horse battery staple"
    [ "$status" -ne 0 ]
    [[ "$output" == *"known tools"* ]]
}
