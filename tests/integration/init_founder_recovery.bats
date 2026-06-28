#!/usr/bin/env bats
# JOY-01CA-AF: a Joy project must have a founding member (the root of the
# attestation chain). `joy project member add` attests new members with the
# CALLER's key, so a member-less project cannot bootstrap one. `joy init` must
# therefore (1) fail fast when no founding identity is available rather than
# create an unrecoverable member-less project, and (2) self-heal an existing
# member-less project (e.g. one an older Joy created before a git identity was
# set) on the next `joy init`.

load setup

@test "joy init fails fast without a git identity and leaves nothing behind" {
    # setup() configured a git identity; remove it to model a fresh repo whose
    # author never ran `git config`. HOME is isolated to TEST_DIR, so there is no
    # global identity to fall back on either.
    git config --unset user.email
    git config --unset user.name || true

    run joy init --name "Late Identity"
    [ "$status" -ne 0 ]
    # The error names the fix (set git user.email or pass --user).
    [[ "$output" == *"user.email"* ]]
    # Fail-fast must not leave a half-initialized project on disk.
    [ ! -d .joy ]
}

@test "joy init succeeds once a git identity is set (the documented recovery)" {
    git config --unset user.email
    git config --unset user.name || true
    run joy init --name "Late Identity"
    [ "$status" -ne 0 ]
    [ ! -d .joy ]

    # Set an identity and retry: a clean fresh init with the founder registered.
    git config user.email "test@example.com"
    git config user.name "Test User"
    run joy init --name "Late Identity"
    [ "$status" -eq 0 ]
    grep -q "test@example.com" .joy/project.yaml
}

@test "joy init --user registers the founder without any git identity" {
    git config --unset user.email
    git config --unset user.name || true

    run joy init --name "By Flag" --user "founder@example.com"
    [ "$status" -eq 0 ]
    grep -q "founder@example.com" .joy/project.yaml
}

@test "joy init self-heals a member-less project on re-init" {
    # A normal init records the founder (test@example.com from setup).
    joy init --name "Healme" >/dev/null

    # Model a legacy member-less project by dropping the members mapping
    # (everything from `members:` up to the next top-level key `created:`).
    awk '/^members:/{skip=1} /^created:/{skip=0} !skip' .joy/project.yaml > .joy/project.yaml.tmp
    mv .joy/project.yaml.tmp .joy/project.yaml
    ! grep -q "test@example.com" .joy/project.yaml

    # Re-running init registers the founder from the available git identity.
    run joy init
    [ "$status" -eq 0 ]
    [[ "$output" == *"founding member"* ]]
    grep -q "test@example.com" .joy/project.yaml
}

@test "joy init warns when re-init cannot heal a member-less project" {
    joy init --name "Healme" >/dev/null
    awk '/^members:/{skip=1} /^created:/{skip=0} !skip' .joy/project.yaml > .joy/project.yaml.tmp
    mv .joy/project.yaml.tmp .joy/project.yaml

    # No identity available and none on disk: re-init sets up the local
    # environment but cannot register a founder, so it warns instead of healing.
    git config --unset user.email
    git config --unset user.name || true
    run joy init
    [ "$status" -eq 0 ]
    [[ "$output" == *"no founding member"* ]]
    ! grep -q "@example.com" .joy/project.yaml
}
