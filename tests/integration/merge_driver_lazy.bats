#!/usr/bin/env bats
# JOY-0162: every joy invocation reasserts the YAML merge driver
# registration so a binary upgrade alone is enough to re-protect
# existing repos.

load setup

@test "joy ls re-creates .gitattributes block when it has been deleted" {
    joy init --name "Test"
    [ -f ".gitattributes" ]
    rm -f .gitattributes
    [ ! -f ".gitattributes" ]
    joy ls >/dev/null
    [ -f ".gitattributes" ]
    grep -q "merge=joy-yaml" .gitattributes
    grep -q "merge=union" .gitattributes
}

@test "joy ls re-registers merge.joy-yaml.driver when git config has been wiped" {
    joy init --name "Test"
    git config --local --unset merge.joy-yaml.name
    git config --local --unset merge.joy-yaml.driver
    [ -z "$(git config --local --get merge.joy-yaml.driver || true)" ]
    joy ls >/dev/null
    [ "$(git config --local merge.joy-yaml.name)" = "Joy YAML merge driver" ]
    git config --local merge.joy-yaml.driver | grep -q "joy merge driver"
}

@test "second joy invocation in a clean state does not dirty the working tree" {
    joy init --name "Test"
    git add -A && git commit -m "init [no-item]" --quiet
    [ -z "$(git status --porcelain)" ]
    joy ls >/dev/null
    [ -z "$(git status --porcelain)" ]
}
