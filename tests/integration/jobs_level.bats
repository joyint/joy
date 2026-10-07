#!/usr/bin/env bats
# A job is released at a level (JI-019D-46): the one it asks for, at most
# what the project allows its AI assignee. Approving writes it into the
# job, and from there it is the job's own.

load setup

# One scope item, a job over it, assigned to an AI member the project
# allows `confirmed`; sets JOB_ID. $1: extra arguments for `joy add job`.
make_job_for_claude() {
    joy project member add claude --capabilities implement create --level confirmed \
        --passphrase "$TEST_PASSPHRASE" >/dev/null
    joy add task "Scoped work" >/dev/null
    local scope
    scope=$(joy ls 2>/dev/null | grep "Scoped work" | awk '{print $1}')
    # shellcheck disable=SC2086
    joy add job "Deliver scoped work" "$scope" $1 >/dev/null
    JOB_ID=$(joy ls -J 2>/dev/null | grep "Deliver scoped work" | awk '{print $1}')
    joy assign "$JOB_ID" claude >/dev/null
}

@test "approving a job that names no level writes in what the project allows its assignee" {
    setup_human_auth
    make_job_for_claude ""
    run joy approve "$JOB_ID"
    [ "$status" -eq 0 ]
    run joy show "$JOB_ID" --json
    echo "$output" | jq -e '.data["interaction-level"] == "confirmed"' >/dev/null
}

@test "a job may ask for less than its assignee is allowed" {
    setup_human_auth
    make_job_for_claude "--level proposing"
    run joy approve "$JOB_ID"
    [ "$status" -eq 0 ]
    run joy show "$JOB_ID"
    [[ "$output" == *"Interaction level:"*"proposing"* ]]
}

@test "a job that asks for more than its assignee is allowed is not approved" {
    setup_human_auth
    make_job_for_claude "--level autonomous"
    run joy approve "$JOB_ID"
    [ "$status" -ne 0 ]
    [[ "$output" == *"asks for autonomous"*"allows claude at most confirmed"* ]]
    run joy show "$JOB_ID" --json
    echo "$output" | jq -e '.data.status == "new"' >/dev/null
}

@test "a maximum lowered after the approval does not reach into the job" {
    setup_human_auth
    make_job_for_claude ""
    joy approve "$JOB_ID"
    joy project member edit claude --project --level proposing --passphrase "$TEST_PASSPHRASE"
    run joy show "$JOB_ID" --json
    echo "$output" | jq -e '.data["interaction-level"] == "confirmed"' >/dev/null
}

@test "the assignee starts its own job and hands it to review without the jobs capability" {
    setup_human_auth
    make_job_for_claude ""
    joy approve "$JOB_ID"
    setup_ai_session claude
    run joy start "$JOB_ID"
    [ "$status" -eq 0 ]
    run joy status "$JOB_ID" review
    [ "$status" -eq 0 ]
    # accepting it is not the assignee's
    run joy status "$JOB_ID" closed
    [ "$status" -ne 0 ]
}

@test "--level is for a job" {
    setup_human_auth
    run joy add task "Not a job" --level confirmed
    [ "$status" -ne 0 ]
    [[ "$output" == *"--level is for a job"* ]]
}
