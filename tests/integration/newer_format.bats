#!/usr/bin/env bats
# A project written by a newer joy is named in one sentence, with what to
# do, before anything of it is read. 0.22 died on a parse error when a
# shared project was brought over by a newer joy (2026-10-09).

load setup

@test "a project from a newer joy says one sentence: update joy" {
    setup_human_auth
    joy add task "Readable today" >/dev/null
    # what a later joy will write first
    sed -i "s/^format: [0-9]*$/format: 99/" .joy/project.yaml
    grep -q "^format: 99$" .joy/project.yaml

    run --separate-stderr joy ls
    [ "$status" -eq 1 ]
    [ "$output" = "" ]
    [ "$stderr" = "Error: this project needs a newer joy: install the update" ]

    run --separate-stderr joy add task "Nothing is written either"
    [ "$status" -eq 1 ]
    [[ "$stderr" == *"install the update"* ]]
    [[ "$stderr" != *"Caused by"* ]]
    [ "$(ls .joy/items | wc -l)" -eq 1 ]
}

@test "a project in member files says its format first, one from before says none" {
    setup_human_auth
    [ "$(head -1 .joy/project.yaml)" = "format: 2" ]
}
