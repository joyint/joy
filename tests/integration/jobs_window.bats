#!/usr/bin/env bats
# A job is an item with a window (JOY-02C9-70): the window's end is
# called `until` in the job file, on the command line and in `joy show`,
# and a job file from before, which says `deadline`, reads the same. Its
# priority and effort are the item's: set at `joy add`, changed with
# `joy edit`, like on any item.
#
# `until` is a change of the job file an older joy reads wrong (it shows
# the job without its end and drops the end at its next write), so the
# first job that says it lifts the project's format to 3 in the same
# write (JOY-02CA-EE); a project without one stays at 2.

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

# A project of joy 0.23 (format 2) is lifted to 3 by the first job that
# says `until`: joy-core's store tests build one. A project joy writes
# today is at 4 from its first member file, and a job changes nothing.
@test "a job that says until leaves a project of format 4 as it is" {
    setup_human_auth
    make_job
    [ "$(head -1 .joy/project.yaml)" = "format: 4" ]
    joy edit "$JOB_ID" --not-before 2026-12-01
    git add -A && git commit -qm "before [no-item]"

    joy edit "$JOB_ID" --until 2026-12-24
    [ "$(head -1 .joy/project.yaml)" = "format: 4" ]
    run joy show "$JOB_ID"
    [ "$status" -eq 0 ]
    [[ "$output" == *"until 2026-12-24"* ]]
    # saving the project keeps the number
    joy project set name "Renamed"
    [ "$(head -1 .joy/project.yaml)" = "format: 4" ]
}

@test "a job file from before says until when it is written again" {
    setup_human_auth
    make_job
    # as a joy before wrote it: `deadline`
    joy edit "$JOB_ID" --not-before 2026-12-01
    sed -i 's/^\( *\)not_before: \(.*\)$/\1not_before: \2\n\1deadline: 2026-12-24T00:00:00Z/' "$JOB_FILE"
    grep -q "deadline: 2026-12-24" "$JOB_FILE"
    joy edit "$JOB_ID" --max-tokens 5
    grep -q "until: 2026-12-24" "$JOB_FILE"
    ! grep -q "deadline:" "$JOB_FILE"
    [ "$(head -1 .joy/project.yaml)" = "format: 4" ]
}
