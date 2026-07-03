#!/usr/bin/env bats
# joy ai jobs / joy ai agents list the git-native .joy/ai records (JOY-01ED).

load setup

@test "joy ai jobs and joy ai agents list git-native records" {
    joy init --name "Jobs Test" 2>/dev/null

    run joy ai jobs
    [ "$status" -eq 0 ]
    [[ "$output" == *"No AI jobs."* ]]

    # A hand-written job record shows up in the listing.
    mkdir -p .joy/ai/jobs .joy/ai/agents
    cat > .joy/ai/jobs/abc123.yaml <<YAML
id: abc123
item: JT-0001
type: implement
actor: ai:claude@joy
delegated_by: horst@example.com
status: awaiting-approval
created: 2026-07-04T00:00:00Z
updated: 2026-07-04T00:00:01Z
result: proposal ready
YAML
    run joy ai jobs
    [ "$status" -eq 0 ]
    [[ "$output" == *"abc123"* ]]
    [[ "$output" == *"awaiting-approval"* ]]
    [[ "$output" == *"ai:claude@joy"* ]]

    run joy ai jobs --item JT-0001
    [[ "$output" == *"abc123"* ]]
    run joy ai jobs --item OTHER
    [[ "$output" == *"No AI jobs."* ]]

    cat > .joy/ai/agents/ai-claude-joy.yaml <<YAML
member: ai:claude@joy
adapter: mock
model: claude-sonnet-4
YAML
    run joy ai agents
    [ "$status" -eq 0 ]
    [[ "$output" == *"ai:claude@joy"* ]]
    [[ "$output" == *"mock"* ]]
}
