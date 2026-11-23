#!/usr/bin/env bats
# The job of an AI member runs on a platform, on a token its opener
# issues (JI-01A7-0F). A platform is a member of the project: a manager
# adds it by its https address, joy fetches its public key from
# /.well-known/joy-platform-key and the manager signs for it. Opening
# the job issues a token for its assignee (seven days, this one job)
# and seals it for the platform; `joy job revoke` takes the job back.
#
# The platform here is a local HTTPS server (`start_platform` in
# setup.bash), so the cases that fetch its key run on Linux only.

load setup

# A project with the founder, the AI member claude (jobs included) the
# founder delegated to, and a job for claude; sets JOB_ID and JOB_FILE.
project_with_a_job() {
    setup_human_auth
    joy project member add claude -c implement,create,jobs --passphrase "$TEST_PASSPHRASE"
    joy auth token add claude --passphrase "$TEST_PASSPHRASE" >/dev/null
    joy add task "Scoped work"
    local scope
    scope=$(joy ls 2>/dev/null | grep "Scoped work" | awk '{print $1}')
    joy add job "Deliver scoped work" "$scope"
    JOB_ID=$(joy ls -J 2>/dev/null | grep "Deliver scoped work" | awk '{print $1}')
    JOB_FILE=$(find .joy/jobs -name "$JOB_ID-*.yaml")
    joy assign "$JOB_ID" claude
}

@test "a platform is added by its https address, with the key it serves" {
    setup_human_auth
    start_platform one
    run joy project member add --platform "https://$PLATFORM" --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Added platform $PLATFORM"* ]]
    [[ "$output" == *"$PLATFORM_KEY"* ]]
    local file
    file=$(grep -l "^type: platform" .joy/members/*.yaml)
    grep -q "verify_key: $PLATFORM_KEY" "$file"
    grep -q "^- jobs" "$file"
    grep -q "signature:" "$file"
    # members stand in their own blocks
    run joy project member
    [[ "$output" == *"Persons:"* ]]
    [[ "$output" == *"Platforms:"* ]]
    [[ "$output" != *"AI members:"* ]]
    # a second time it is there already
    run joy project member add --platform "https://$PLATFORM" --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"already exists"* ]]
}

@test "a platform is not added over http, and not when it serves no key" {
    setup_human_auth
    run joy project member add --platform "http://localhost:1" --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"a platform is added by its https address"* ]]
    start_platform one
    rm "$TEST_DIR/platform-one/.well-known/joy-platform-key"
    run joy project member add --platform "https://$PLATFORM" --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"answered 404"* ]]
    ! grep -q "^type: platform" .joy/members/*.yaml
}

@test "nothing is assigned to a platform and nobody delegates to one" {
    setup_human_auth
    start_platform one
    joy project member add --platform "https://$PLATFORM" --passphrase "$TEST_PASSPHRASE"
    joy add task "Some work"
    local id
    id=$(joy ls 2>/dev/null | grep "Some work" | awk '{print $1}')
    run joy assign "$id" "$PLATFORM"
    [ "$status" -ne 0 ]
    [[ "$output" == *"platform member"* ]]
    run joy auth token add "$PLATFORM" --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
}

@test "without a platform a job of an AI member cannot be opened" {
    project_with_a_job
    run joy approve "$JOB_ID" --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"sign in on a platform first"* ]]
    run joy show "$JOB_ID" --json
    echo "$output" | jq -e '.data.status == "new"' >/dev/null
}

@test "opening a job issues its token, sealed for the platform, and lifts the format" {
    project_with_a_job
    start_platform one
    joy project member add --platform "https://$PLATFORM" --passphrase "$TEST_PASSPHRASE"
    [ "$(head -1 .joy/project.yaml)" = "format: 4" ]
    run joy approve "$JOB_ID" --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    grep -q "runs_on: $PLATFORM" "$JOB_FILE"
    grep -q "platform: $PLATFORM" "$JOB_FILE"
    grep -q "opener: $FOUNDER_EMAIL" "$JOB_FILE"
    grep -q "sealed: [0-9a-f]\{100,\}" "$JOB_FILE"
    grep -q "expires: " "$JOB_FILE"
    # the token itself is nowhere in clear
    ! grep -q "joy_t_" "$JOB_FILE"
    [ "$(head -1 .joy/project.yaml)" = "format: 5" ]
    # joy ls shows where the job runs
    run joy ls -J
    [[ "$output" == *"PLATFORM"* ]]
    [[ "$output" == *"$PLATFORM"* ]]
}

@test "a job of a person needs no platform and no token" {
    setup_human_auth
    joy add task "Scoped work"
    local scope
    scope=$(joy ls 2>/dev/null | grep "Scoped work" | awk '{print $1}')
    joy add job "By hand" "$scope"
    JOB_ID=$(joy ls -J 2>/dev/null | grep "By hand" | awk '{print $1}')
    joy assign "$JOB_ID" "$FOUNDER_EMAIL"
    run joy approve "$JOB_ID"
    [ "$status" -eq 0 ]
    ! grep -q "runs_on\|sealed" "$(find .joy/jobs -name "$JOB_ID-*.yaml")"
    [ "$(head -1 .joy/project.yaml)" = "format: 4" ]
}

@test "with two platforms the one that runs the job is named" {
    project_with_a_job
    start_platform one
    local first="$PLATFORM"
    joy project member add --platform "https://$PLATFORM" --passphrase "$TEST_PASSPHRASE"
    start_platform two
    joy project member add --platform "https://$PLATFORM" --passphrase "$TEST_PASSPHRASE"
    run joy approve "$JOB_ID" --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"more than one platform"* ]]
    run joy approve "$JOB_ID" --platform "$first" --passphrase "$TEST_PASSPHRASE"
    [ "$status" -eq 0 ]
    grep -q "runs_on: $first" "$JOB_FILE"
}

@test "without a delegation to the assignee joy says so" {
    setup_human_auth
    joy project member add claude -c implement,create,jobs --passphrase "$TEST_PASSPHRASE"
    start_platform one
    joy project member add --platform "https://$PLATFORM" --passphrase "$TEST_PASSPHRASE"
    joy add task "Scoped work"
    local scope
    scope=$(joy ls 2>/dev/null | grep "Scoped work" | awk '{print $1}')
    joy add job "Deliver scoped work" "$scope"
    JOB_ID=$(joy ls -J 2>/dev/null | grep "Deliver scoped work" | awk '{print $1}')
    joy assign "$JOB_ID" claude
    run joy approve "$JOB_ID" --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"you have not delegated to claude yet"* ]]
}

@test "joy job revoke takes a job back: token and platform gone, status new, a comment" {
    project_with_a_job
    start_platform one
    joy project member add --platform "https://$PLATFORM" --passphrase "$TEST_PASSPHRASE"
    joy approve "$JOB_ID" --passphrase "$TEST_PASSPHRASE"
    run joy job revoke "$JOB_ID"
    [ "$status" -eq 0 ]
    [[ "$output" == *"taken back from $PLATFORM"* ]]
    ! grep -q "runs_on\|sealed" "$JOB_FILE"
    run joy show "$JOB_ID" --json
    echo "$output" | jq -e '.data.status == "new"' >/dev/null
    echo "$output" | jq -e --arg p "Taken back from $PLATFORM." '.data.comments[-1].text == $p' >/dev/null
    # a job that runs nowhere has nothing to take back
    run joy job revoke "$JOB_ID"
    [ "$status" -ne 0 ]
    [[ "$output" == *"runs on no platform"* ]]
}

@test "a job sent back to new loses its token and keeps where it would run" {
    project_with_a_job
    start_platform one
    joy project member add --platform "https://$PLATFORM" --passphrase "$TEST_PASSPHRASE"
    joy approve "$JOB_ID" --passphrase "$TEST_PASSPHRASE"
    joy status "$JOB_ID" new
    ! grep -q "sealed" "$JOB_FILE"
    grep -q "runs_on: $PLATFORM" "$JOB_FILE"
    # opened again it gets a fresh token
    joy approve "$JOB_ID" --passphrase "$TEST_PASSPHRASE"
    grep -q "sealed: " "$JOB_FILE"
}
