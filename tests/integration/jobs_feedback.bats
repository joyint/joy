#!/usr/bin/env bats
# Feedback axis on jobs (JOY-020C-A3): job.feedback is the dialog axis,
# orthogonal to status like validity on decisions. Set and cleared via
# `joy edit <JOB> --feedback awaited|received|none`, shown in the FBK
# column of `ls -J` and as a Feedback line in `joy show`.

load setup

# Create one scope item plus a job over it; sets SCOPE_ID and JOB_ID.
make_job() {
    joy add task "Scoped work"
    SCOPE_ID=$(joy ls 2>/dev/null | grep "Scoped work" | awk '{print $1}')
    joy add job "Deliver scoped work" "$SCOPE_ID"
    JOB_ID=$(joy ls -J 2>/dev/null | grep "Deliver scoped work" | awk '{print $1}')
}

@test "--feedback awaited sets the dialog field" {
    setup_human_auth
    make_job
    joy edit "$JOB_ID" --feedback awaited
    run joy show "$JOB_ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.job.feedback == "awaited"' >/dev/null
}

@test "--feedback received replaces awaited" {
    setup_human_auth
    make_job
    joy edit "$JOB_ID" --feedback awaited
    joy edit "$JOB_ID" --feedback received
    run joy show "$JOB_ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.job.feedback == "received"' >/dev/null
}

@test "--feedback none clears the dialog field" {
    setup_human_auth
    make_job
    joy edit "$JOB_ID" --feedback awaited
    joy edit "$JOB_ID" --feedback none
    run joy show "$JOB_ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.job.feedback == null' >/dev/null
}

@test "--feedback rejects an unknown value" {
    setup_human_auth
    make_job
    run joy edit "$JOB_ID" --feedback maybe
    [ "$status" -ne 0 ]
    [[ "$output" == *"unknown feedback"* ]]
}

@test "--feedback is rejected on non-job items" {
    setup_human_auth
    joy add task "Plain task"
    ID=$(joy ls 2>/dev/null | grep "Plain task" | awk '{print $1}')
    run joy edit "$ID" --feedback awaited
    [ "$status" -ne 0 ]
    [[ "$output" == *"only valid for job items"* ]]
    run joy show "$ID" --json
    echo "$output" | jq -e '.data.job == null' >/dev/null
}
