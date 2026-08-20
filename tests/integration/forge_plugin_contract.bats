#!/usr/bin/env bats
#
# The forge-plugin contract (JOY-0252-1A): every plugin is a binary that
# answers three questions on stdout as JSON, and joy-core and the platform
# both ask exactly these. Driven here the way they drive it, so a plugin
# that changes its answers is caught before a host notices.
#
# `claims` decides responsibility, `resolve` is PURE (an address alone,
# no network, no config), `identity` is the only question that may look
# outward, and there the forge CLI or curl is the marked stub.

load setup

@test "claims: each plugin answers for its own domains and no others" {
    run -0 joy-github claims --remote git@github.com:example/r.git
    [ "$output" = '{"claims":true}' ]
    run -0 joy-github claims --remote https://github.com/example/r.git
    [ "$output" = '{"claims":true}' ]
    # a lookalike is not the product domain
    run -0 joy-github claims --remote git@github.com.evil.example:example/r.git
    [ "$output" = '{"claims":false}' ]
    run -0 joy-github claims --remote git@gitlab.com:example/r.git
    [ "$output" = '{"claims":false}' ]

    run -0 joy-gitlab claims --remote git@gitlab.com:example/r.git
    [ "$output" = '{"claims":true}' ]
    run -0 joy-gitlab claims --remote git@github.com:example/r.git
    [ "$output" = '{"claims":false}' ]

    # a remote with no host at all is nobody's business
    run -0 joy-github claims --remote /srv/local/repo.git
    [ "$output" = '{"claims":false}' ]
}

@test "claims: gitea answers for the instances tea is signed in to" {
    export TEA_CONFIG_DIR="$TEST_DIR/tea-config"
    mkdir -p "$TEA_CONFIG_DIR"
    printf 'logins:\n- name: codeberg\n  url: https://codeberg.org/\n  user: alice\n' \
        > "$TEA_CONFIG_DIR/config.yml"

    run -0 joy-gitea claims --remote git@codeberg.org:example/r.git
    [ "$output" = '{"claims":true}' ]
    # a self-hosted instance nobody is signed in to stays unclaimed; the
    # project.yaml forge override is its road, not a guess by hostname
    run -0 joy-gitea claims --remote https://gitea.example.com/o/r.git
    [ "$output" = '{"claims":false}' ]
    run -0 joy-gitea claims --remote git@github.com:o/r.git
    [ "$output" = '{"claims":false}' ]
}

@test "resolve is pure: the address alone decides, and it never vouches" {
    # GitHub's two forms, current and legacy
    run -0 joy-github resolve --email 777+alice-login@users.noreply.github.com
    [[ "$output" == *'"known":true'* ]]
    [[ "$output" == *'"login":"alice-login"'* ]]
    [[ "$output" == *'"user_id":"777"'* ]]
    # pure means: no addresses are claimed here, that is identity's job
    [[ "$output" == *'"emails":[]'* ]]

    run -0 joy-github resolve --email alice-login@users.noreply.github.com
    [[ "$output" == *'"login":"alice-login"'* ]]
    [[ "$output" == *'"user_id":null'* ]]

    # GitLab writes a dash where GitHub writes a plus
    run -0 joy-gitlab resolve --email 4711-alice@users.noreply.gitlab.com
    [[ "$output" == *'"login":"alice"'* ]]
    [[ "$output" == *'"user_id":"4711"'* ]]

    # Gitea's shape carries no id
    run -0 joy-gitea resolve --email alice-login@noreply.codeberg.org
    [[ "$output" == *'"login":"alice-login"'* ]]
    [[ "$output" == *'"user_id":null'* ]]
}

@test "resolve: an ordinary address is honestly unknown" {
    for plugin in joy-github joy-gitlab joy-gitea; do
        run -0 "$plugin" resolve --email plain@example.com
        [[ "$output" == *'"known":false'* ]]
    done
}

@test "identity: the token is named by variable, and the answer carries the addresses" {
    # curl is the forge boundary here; the stub answers only when the
    # Bearer header actually arrived
    STUB_DIR="$TEST_DIR/stub-bin"
    mkdir -p "$STUB_DIR"
    cat > "$STUB_DIR/curl" <<'STUB'
#!/bin/sh
for arg in "$@"; do
    case "$arg" in
    "Authorization: Bearer s3cr3t-value") echo '[{"email":"alice@example.com","verified":true}]'; exit 0 ;;
    esac
done
exit 22
STUB
    chmod +x "$STUB_DIR/curl"
    export PATH="$STUB_DIR:$PATH"
    export GH_TOKEN="s3cr3t-value"

    run -0 joy-github identity --login alice-login --user-id 777 --token-env GH_TOKEN
    [[ "$output" == *'"known":true'* ]]
    [[ "$output" == *'"login":"alice-login"'* ]]
    [[ "$output" == *'alice@example.com'* ]]

    # a variable that holds nothing yields an honest empty answer rather
    # than a guess
    run -0 joy-github identity --login alice-login --token-env NOT_SET_ANYWHERE
    [[ "$output" != *'alice@example.com'* ]]
}
