#!/usr/bin/env bats
# JOY-01BF-2E: `joy init --anonymous` starts a project in anonymous privacy mode
# (ADR-042). The founder is recorded under an opaque id from the very first
# written file, so the git e-mail never reaches a written -- let alone
# committed -- .joy file. Unlike the open->anonymous transition, this is the
# greenfield path: there is no prior open state and no e-mail in git history.

load setup

TEST_EMAIL="test@example.com"

@test "joy init --anonymous records the founder under an opaque id, never the e-mail" {
    JOY_PASSPHRASE="$TEST_PASSPHRASE" joy init --anonymous --name "Secret" >/dev/null

    # No member e-mail in any generated .joy file (the encrypted members.yaml
    # is scanned too: -I is deliberately omitted).
    run grep -rl "$TEST_EMAIL" .joy/
    [ "$status" -ne 0 ]

    # The member map is keyed by an opaque id and the mode is anonymous.
    run grep -qE '^  m-[a-z2-7]{10}:' .joy/project.yaml
    [ "$status" -eq 0 ]
    run grep -q "privacy: anonymous" .joy/project.yaml
    [ "$status" -eq 0 ]

    # members.yaml exists and is an encrypted JOYCRYPT blob.
    [ -f .joy/members.yaml ]
    run head -c 8 .joy/members.yaml
    [[ "$output" == "JOYCRYPT" ]]
}

@test "joy init --anonymous leaves the founder authenticated for immediate work" {
    JOY_PASSPHRASE="$TEST_PASSPHRASE" joy init --anonymous --name "Secret" >/dev/null

    # The initial session is active, so a write succeeds without a separate
    # joy auth, and it still records no e-mail.
    run joy add task "first"
    [ "$status" -eq 0 ]
    run grep -rl "$TEST_EMAIL" .joy/
    [ "$status" -ne 0 ]
}

@test "joy init --anonymous keeps the e-mail out of git history after commit" {
    JOY_PASSPHRASE="$TEST_PASSPHRASE" joy init --anonymous --name "Secret" >/dev/null
    git add -A
    git commit -q --no-verify -m "initial anonymous project"

    # Search every blob across all of history; the e-mail must appear nowhere.
    run bash -c 'git grep -I "'"$TEST_EMAIL"'" $(git rev-list --all) -- 2>/dev/null'
    [ -z "$output" ]
}

@test "anonymous: adding a human member is refused rather than leaking the e-mail" {
    JOY_PASSPHRASE="$TEST_PASSPHRASE" joy init --anonymous --name "Secret" >/dev/null

    run env JOY_PASSPHRASE="$TEST_PASSPHRASE" joy project member add dev@example.com \
        --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"anonymous"* ]]

    # The e-mail never reached project.yaml.
    run grep -c "dev@example.com" .joy/project.yaml
    [ "$output" = "0" ]
}

@test "anonymous: --json resolves members like the terminal, never a raw id" {
    JOY_PASSPHRASE="$TEST_PASSPHRASE" joy init --anonymous --name "Secret" >/dev/null
    id=$(joy add task "x" | grep -oiE '[A-Z]+-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{2}' | head -1)

    # Every --json surface resolves to the e-mail; none exposes a raw opaque id.
    out=$(joy project --json; joy project member --json; joy show "$id" --json; \
          joy log --json; joy ls --json)
    run grep -cE 'm-[a-z2-7]{10}' <<<"$out"
    [ "$output" = "0" ]
    [[ "$out" == *"$TEST_EMAIL"* ]]

    # On disk the id stays raw: anonymous at rest, resolved only on output.
    run grep -cE '^  m-[a-z2-7]{10}:' .joy/project.yaml
    [ "$output" -ge 1 ]
}

@test "joy init --anonymous requires a passphrase (no silent open fallback)" {
    # No passphrase available and no TTY: identity setup must fail rather than
    # quietly creating an open, e-mail-bearing project.
    run env -u JOY_PASSPHRASE joy init --anonymous --name "Secret" </dev/null
    [ "$status" -ne 0 ]

    # Nothing was scaffolded: the failure happens before any .joy file is written.
    [ ! -d .joy ]
}
