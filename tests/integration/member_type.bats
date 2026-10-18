#!/usr/bin/env bats
# A member file says what its member is (JOY-02DB-09): `type` decides,
# joy names a file without one instead of guessing, and a platform member
# is kept but takes no work and no delegation yet.

load setup

TEST_EMAIL="test@example.com"

# Put a member file in by hand and list its id in project.yaml.
put_member() {
    local id="$1" body="$2"
    printf '%s' "$body" > ".joy/members/$id.yaml"
    awk -v id="$id" '/^members:/ { print; print "- " id; next } { print }' \
        .joy/project.yaml > .joy/project.yaml.tmp \
        && mv .joy/project.yaml.tmp .joy/project.yaml
}

@test "a member file says its type and the name its member goes by" {
    setup_human_auth
    file=$(member_file "$TEST_EMAIL")
    [ -n "$file" ]
    [ "$(head -1 "$file")" = "type: person" ]
    [ "$(sed -n 2p "$file")" = "identity:" ]
    [ "$(sed -n 3p "$file")" = "  $TEST_EMAIL:" ]
    grep -q '^    seed_wrap: ' "$file"
    ! grep -q 'seed_wrap_passphrase' "$file"
    ! grep -q '^email:' "$file"
}

@test "a member file without a type is named, not guessed at" {
    setup_human_auth
    put_member m-nonenonen $'identity:\n  someone@example.com: {}\ncapabilities: all\nupdated: 2026-01-01T00:00:00Z\n'

    run --separate-stderr joy ls
    [ "$status" -ne 0 ]
    [[ "$stderr" == *"m-nonenonen"* ]]
    [[ "$stderr" == *"type"* ]]
}

@test "a platform member is kept and takes no work and no token" {
    setup_human_auth
    joy add task "Work for somebody" >/dev/null
    item=$(ls .joy/items | head -1 | cut -d- -f1-3)
    put_member m-platformm $'type: platform\nidentity:\n  joyint.com: {}\ncapabilities:\n- implement\nupdated: 2026-01-01T00:00:00Z\n'

    run joy project member
    [ "$status" -eq 0 ]

    run joy assign "$item" joyint.com
    [ "$status" -ne 0 ]
    [[ "$output" == *"platform member"* ]]

    run joy auth token add joyint.com --passphrase "$TEST_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not an AI member"* ]]
}
