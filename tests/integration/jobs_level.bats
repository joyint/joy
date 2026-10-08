#!/usr/bin/env bats
# A job and its assignee (JI-019D-46). A job is written with its assignee
# and its level, and its assignee takes it once it is set to open. The
# assignee holds the jobs capability for the person, and the level is at
# most the one it may run at for them. joy checks that when the job is
# assigned and again when it is set to open, because a member can be
# changed in between; from then on the level is the job's own.

load setup

# One scope item and a job over it; sets JOB_ID. The project allows the
# AI member claude `confirmed`. $1: extra arguments for `joy add job`,
# $2: claude's capabilities (default: it may take jobs).
make_job() {
    # shellcheck disable=SC2086
    joy project member add claude --capabilities ${2:-implement create jobs} --level confirmed \
        --passphrase "$TEST_PASSPHRASE" >/dev/null
    joy add task "Scoped work" >/dev/null
    local scope
    scope=$(joy ls 2>/dev/null | grep "Scoped work" | awk '{print $1}')
    # shellcheck disable=SC2086
    joy add job "Deliver scoped work" "$scope" $1 >/dev/null
    JOB_ID=$(joy ls -J 2>/dev/null | grep "Deliver scoped work" | awk '{print $1}')
}

make_job_for_claude() {
    make_job "$1"
    joy assign "$JOB_ID" claude >/dev/null
}

@test "a job that names no level gets its assignee's when it is set to open" {
    setup_human_auth
    make_job_for_claude ""
    run joy status "$JOB_ID" open
    [ "$status" -eq 0 ]
    run joy show "$JOB_ID" --json
    echo "$output" | jq -e '.data["interaction-level"] == "confirmed"' >/dev/null
}

@test "a job may ask for less than its assignee may run at" {
    setup_human_auth
    make_job_for_claude "--level proposing"
    run joy status "$JOB_ID" open
    [ "$status" -eq 0 ]
    run joy show "$JOB_ID"
    [[ "$output" == *"Interaction level:"*"proposing"* ]]
}

@test "a job that asks for more than a member may run at is not given to it" {
    setup_human_auth
    make_job "--level autonomous"
    run joy assign "$JOB_ID" claude
    [ "$status" -ne 0 ]
    [[ "$output" == *"asks for autonomous"*"claude may run at most at confirmed for you"* ]]
    run joy show "$JOB_ID" --json
    echo "$output" | jq -e '.data.assignees == null or (.data.assignees | length == 0)' >/dev/null
}

@test "a job is not given to a member without the jobs capability" {
    setup_human_auth
    make_job "" "implement create"
    run joy assign "$JOB_ID" claude
    [ "$status" -ne 0 ]
    [[ "$output" == *"claude does not hold the jobs capability for you"* ]]
}

@test "a member changed after the job was written is refused when the job is set to open" {
    setup_human_auth
    make_job_for_claude "--level confirmed"
    # the project lowers what claude may run at
    joy project member edit claude --project --level proposing --passphrase "$TEST_PASSPHRASE"
    run joy status "$JOB_ID" open
    [ "$status" -ne 0 ]
    [[ "$output" == *"claude may run at most at proposing for you"* ]]
    run joy show "$JOB_ID" --json
    echo "$output" | jq -e '.data.status == "new"' >/dev/null
    # ...and takes the jobs capability away
    joy project member edit claude --project --level confirmed --rm-capability jobs \
        --passphrase "$TEST_PASSPHRASE"
    run joy status "$JOB_ID" open
    [ "$status" -ne 0 ]
    [[ "$output" == *"claude does not hold the jobs capability for you"* ]]
}

@test "a level lowered after the job was set to open does not reach into the job" {
    setup_human_auth
    make_job_for_claude ""
    joy status "$JOB_ID" open
    joy project member edit claude --project --level proposing --passphrase "$TEST_PASSPHRASE"
    run joy show "$JOB_ID" --json
    echo "$output" | jq -e '.data["interaction-level"] == "confirmed"' >/dev/null
}

@test "the assignee starts its own job and hands it to review, and does not accept it" {
    setup_human_auth
    make_job_for_claude ""
    joy status "$JOB_ID" open
    setup_ai_session claude
    run joy start "$JOB_ID"
    [ "$status" -eq 0 ]
    run joy status "$JOB_ID" review
    [ "$status" -eq 0 ]
    # accepting the result is a person's step
    run joy status "$JOB_ID" closed
    [ "$status" -ne 0 ]
}

@test "an assignee that lost the jobs capability after the job was set to open does not start it" {
    setup_human_auth
    make_job_for_claude ""
    joy status "$JOB_ID" open
    joy project member edit claude --project --rm-capability jobs --passphrase "$TEST_PASSPHRASE"
    setup_ai_session claude
    run joy start "$JOB_ID"
    [ "$status" -ne 0 ]
    [[ "$output" == *"jobs"* ]]
}

@test "a person who starts an AI member's job orders exactly this job" {
    setup_human_auth
    make_job_for_claude ""
    joy approve "$JOB_ID"
    # not the assignee, a person: the job goes to in-progress for claude
    run joy start "$JOB_ID"
    [ "$status" -eq 0 ]
    run joy show "$JOB_ID" --json
    echo "$output" | jq -e '.data.status == "in-progress"' >/dev/null
    echo "$output" | jq -e '.data.assignees[0].member == "claude"' >/dev/null
}

@test "joy approve is new -> open, and an AI member does not approve a job" {
    setup_human_auth
    make_job_for_claude ""
    setup_ai_session claude
    run joy approve "$JOB_ID"
    [ "$status" -ne 0 ]
    run joy show "$JOB_ID" --json
    echo "$output" | jq -e '.data.status == "new"' >/dev/null
}

@test "--level is for a job" {
    setup_human_auth
    run joy add task "Not a job" --level confirmed
    [ "$status" -ne 0 ]
    [[ "$output" == *"--level is for a job"* ]]
}
