#!/usr/bin/env bats
# JOY-020F-34: joy ai reset must recognize platform-provisioned agents.
# The platform writes .joy/ai/agents/<member>.yaml (joy_core::agents) and
# registers the member in project.yaml, but creates none of the per-tool
# marker paths -- reset used to report "0 tools reset" and leave both the
# agent config and the member behind. A --tool filter selects an agent
# config by canonical member id (ai:<tool>@joy) or by its ACP adapter.

load setup

# Platform-shaped fixture: an agent config written the way the platform
# writes it (joy_core::agents::save_agent), without any tool marker path.
write_agent_config() {
    local member="$1" adapter="$2" stem="$3"
    mkdir -p .joy/ai/agents
    cat > ".joy/ai/agents/${stem}.yaml" <<YAML
member: ${member}
adapter: ${adapter}
model: claude-sonnet-4
YAML
}

@test "joy ai reset removes a platform-provisioned agent config and its member" {
    setup_human_auth
    joy project member add ai:worker@joy --passphrase "$TEST_PASSPHRASE"
    write_agent_config ai:worker@joy claude-code ai-worker-joy

    run joy ai reset --force </dev/null
    [ "$status" -eq 0 ]
    [[ "$output" == *"0 tools reset, 1 agent config removed"* ]]
    [ ! -f .joy/ai/agents/ai-worker-joy.yaml ]
    ! grep -q "ai:worker@joy" .joy/project.yaml
}

@test "joy ai reset --tool matches an agent config by adapter" {
    setup_human_auth
    joy project member add ai:worker@joy --passphrase "$TEST_PASSPHRASE"
    write_agent_config ai:worker@joy claude-code ai-worker-joy

    # A different tool's filter must not touch the platform agent.
    run joy ai reset --tool qwen --force </dev/null
    [ "$status" -eq 0 ]
    [[ "$output" == *"No AI tool configurations found."* ]]
    [ -f .joy/ai/agents/ai-worker-joy.yaml ]
    grep -q "ai:worker@joy" .joy/project.yaml

    # The adapter's tool removes config and member.
    run joy ai reset --tool claude --force </dev/null
    [ "$status" -eq 0 ]
    [[ "$output" == *"1 agent config removed"* ]]
    [ ! -f .joy/ai/agents/ai-worker-joy.yaml ]
    ! grep -q "ai:worker@joy" .joy/project.yaml
}

@test "joy ai reset --tool matches an agent config by canonical member id" {
    setup_human_auth
    joy project member add ai:claude@joy --passphrase "$TEST_PASSPHRASE"
    # The mock adapter maps to no tool; only the member id can match.
    write_agent_config ai:claude@joy mock ai-claude-joy

    run joy ai reset --tool claude --force </dev/null
    [ "$status" -eq 0 ]
    [[ "$output" == *"1 agent config removed"* ]]
    [ ! -f .joy/ai/agents/ai-claude-joy.yaml ]
    ! grep -q "ai:claude@joy" .joy/project.yaml
}

@test "joy ai reset --tool preserves other agents' configs in .joy/ai/" {
    setup_human_auth
    joy project member add ai:worker@joy --passphrase "$TEST_PASSPHRASE"
    joy project member add ai:scout@joy --passphrase "$TEST_PASSPHRASE"
    write_agent_config ai:worker@joy claude-code ai-worker-joy
    write_agent_config ai:scout@joy qwen-code ai-scout-joy

    run joy ai reset --tool claude --force </dev/null
    [ "$status" -eq 0 ]
    [ ! -f .joy/ai/agents/ai-worker-joy.yaml ]
    ! grep -q "ai:worker@joy" .joy/project.yaml
    # The .joy/ai cleanup must not wipe the surviving agent config.
    [ -f .joy/ai/agents/ai-scout-joy.yaml ]
    grep -q "ai:scout@joy" .joy/project.yaml
}

@test "joy ai reset tool-marker behaviour and footer unchanged without agent configs" {
    setup_human_auth
    mkdir -p .claude/skills/joy
    echo "skill" > .claude/skills/joy/SKILL.md

    run joy ai reset --tool claude --force </dev/null
    [ "$status" -eq 0 ]
    [[ "$output" == *"1 tool reset"* ]]
    [[ "$output" != *"agent config"* ]]
    [ ! -d .claude/skills/joy ]
}
