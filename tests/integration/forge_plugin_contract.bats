#!/usr/bin/env bats
#
# The forge-plugin contract (JOY-0252-1A): every plugin is a binary that
# answers three questions on stdout as JSON, and joy-core and the platform
# both ask exactly these. Driven here the way they drive it, so a plugin
# that changes its answers is caught before a host notices.
#
# `claims` decides responsibility, `resolve` is PURE (an address alone,
# no network, no config), `identity` is the only question that may look
# outward, and there the marked stub is the fake forge API on the
# loopback interface (JOY-0298-E4: the connector speaks HTTP itself, so
# there is no curl and no gh in the API path any more).

load setup
load forge_fake

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
    # The forge boundary is the fake API now (design D2.8): the
    # connector speaks HTTP itself, so there is no curl to stub.
    start_fake_forge
    point_forge_at_fake github.com github
    printf 'alice@example.com' > "$FAKE_FORGE_DIR/email"
    export GH_TOKEN="s3cr3t-value"

    run -0 joy-github identity --login alice-login --user-id 777 --token-env GH_TOKEN
    [[ "$output" == *'"known":true'* ]]
    [[ "$output" == *'"login":"alice-login"'* ]]
    [[ "$output" == *'alice@example.com'* ]]
    # the token reached the forge in a header, never in a process list
    grep -q "GET /user/emails" "$FAKE_FORGE_DIR/calls"

    # a variable that holds nothing yields an honest empty answer rather
    # than a guess
    run -0 joy-github identity --login alice-login --token-env NOT_SET_ANYWHERE
    [[ "$output" != *'alice@example.com'* ]]
    stop_fake_forge
}

@test "version: every connector name answers the protocol handshake" {
    # D2.2a: the question is about the BINARY, so no forge id goes in
    # front of it, and the answer names every forge the file carries.
    run -0 joy-forge version
    [[ "$output" == *'"protocol":2'* ]]
    [[ "$output" == *'"github"'* ]]
    [[ "$output" == *'"gitlab"'* ]]
    [[ "$output" == *'"gitea"'* ]]
    for plugin in joy-github joy-gitlab joy-gitea; do
        run -0 "$plugin" version
        [[ "$output" == *'"protocol":2'* ]]
    done
}

@test "claims: an internal host in forges.yaml is claimed with no forge CLI" {
    # D2.5: until now a self hosted host was claimed only when gh, glab
    # or tea was signed in to it, which makes the sign in door circular
    # for an enterprise. An operator's file cuts the circle.
    mkdir -p "$XDG_CONFIG_HOME/joy"
    cat > "$XDG_CONFIG_HOME/joy/forges.yaml" <<'YAML'
- host: git.internal.test
  kind: gitea
  api_base: https://git.internal.test/api/v1
YAML

    run -0 joy-gitea claims --remote git@git.internal.test:team/app.git
    [ "$output" = '{"claims":true}' ]
    # and it stays the Gitea operator's host, not everybody's
    run -0 joy-github claims --remote git@git.internal.test:team/app.git
    [ "$output" = '{"claims":false}' ]
    run -0 joy-gitea claims --remote git@stranger.test:team/app.git
    [ "$output" = '{"claims":false}' ]
}

@test "claims: the project's forge override is consulted too" {
    # D2.5: the project level `forge:` override wins for that project,
    # so a connector claims its remotes even where nothing else does.
    joy init --name "Override" --acronym OV >/dev/null
    run -0 joy project set forge gitea
    run -0 joy-gitea claims --remote https://git.nobody-knows.test/o/r.git
    [ "$output" = '{"claims":true}' ]
}
