#!/usr/bin/env bats
# A job is an item with a window (JOY-02C9-70): the window's end is
# called `until` in the job file, on the command line and in `joy show`,
# and a job file from before, which says `deadline`, reads the same. Its
# priority and effort are the item's: set at `joy add`, changed with
# `joy edit`, like on any item.

load setup

# Create one scope item plus a job over it; sets SCOPE_ID, JOB_ID and
# JOB_FILE.
make_job() {
    joy add task "Scoped work"
    SCOPE_ID=$(joy ls 2>/dev/null | grep "Scoped work" | awk '{print $1}')
    joy add job "Deliver scoped work" "$SCOPE_ID" "$@"
    JOB_ID=$(joy ls -J 2>/dev/null | grep "Deliver scoped work" | awk '{print $1}')
    JOB_FILE=$(find .joy/jobs -name "$JOB_ID-*.yaml")
}

@test "--until sets the window's end, and the job file says until" {
    setup_human_auth
    make_job
    joy edit "$JOB_ID" --until 2026-12-24
    grep -q "until: 2026-12-24" "$JOB_FILE"
    ! grep -q "deadline" "$JOB_FILE"
    run joy show "$JOB_ID"
    [ "$status" -eq 0 ]
    [[ "$output" == *"until 2026-12-24"* ]]
}

@test "--deadline, as typed before, is the same flag" {
    setup_human_auth
    make_job
    joy edit "$JOB_ID" --deadline 2026-12-24
    grep -q "until: 2026-12-24" "$JOB_FILE"
}

@test "a job file from before says deadline and reads as until" {
    setup_human_auth
    make_job
    joy edit "$JOB_ID" --until 2026-12-24
    sed -i 's/^\( *\)until:/\1deadline:/' "$JOB_FILE"
    grep -q "deadline: 2026-12-24" "$JOB_FILE"
    run joy show "$JOB_ID"
    [ "$status" -eq 0 ]
    [[ "$output" == *"until 2026-12-24"* ]]
    # written again, it says until
    joy edit "$JOB_ID" --not-before 2026-12-01
    grep -q "until: 2026-12-24" "$JOB_FILE"
    ! grep -q "deadline" "$JOB_FILE"
}

@test "a job's priority and effort are set and changed like on any item" {
    setup_human_auth
    make_job --priority high --effort 3
    run joy show "$JOB_ID" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.data.priority == "high" and .data.effort == 3' >/dev/null
    joy edit "$JOB_ID" --priority low --effort 1
    run joy show "$JOB_ID" --json
    echo "$output" | jq -e '.data.priority == "low" and .data.effort == 1' >/dev/null
}
