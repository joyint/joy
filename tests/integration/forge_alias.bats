#!/usr/bin/env bats
#
# Forge-connector alias resolution, end to end (epic JOY-0251-AA,
# JOY-0253-8A / JOY-0254-3C, reported as JP-00BF-94): a member enrolled
# under their PRIMARY address keeps working when the clone's git config
# carries GitHub's noreply alias.
#
# Since package J11 that git config is a PREFILL and nothing more (D3.9):
# it is the address joy OFFERS on a machine that has not been told who
# acts. So every case here is a CLONE, and `forget_this_device` is what
# makes it one: the project travels, the device's pin and sessions do
# not, and the address the clone carries is the one that reaches the
# resolution. Without that the pin would answer first and no alias would
# ever be looked at. joy-core resolves via the GitHub
# connector; the connector asks the forge itself over HTTP since
# JOY-0298-E4. The forge boundary (the one thing tests cannot have for
# real) is two MARKED STUBS: gh as a source of a TOKEN (decision 19),
# and the fake forge API on the loopback interface (D2.8). Everything
# else is the real product path: real joy, real connector, real
# project.

load setup
load forge_fake

FOUNDER_PASSPHRASE="correct horse battery staple extra words"
ALICE_PASSPHRASE="alpha bravo charlie delta echo foxtrot"

extract_otp() {
    echo "$1" | sed -n 's/^[[:space:]]*One-time password:[[:space:]]*\([A-Za-z0-9-]*\).*$/\1/p' | head -1
}

# The forge boundary: gh names the login and hands out a TOKEN, and the
# fake API answers the one read the connector makes with it.
install_gh_stub() {
    start_fake_forge
    point_forge_at_fake github.com github
    printf 'alice@example.com' > "$FAKE_FORGE_DIR/email"
    install_gh_token_stub "gho_alias-test-token"
    # gh's config names the signed-in login, offline
    export GH_CONFIG_DIR="$TEST_DIR/gh-config"
    mkdir -p "$GH_CONFIG_DIR"
    printf 'github.com:\n    user: alice-login\n' > "$GH_CONFIG_DIR/hosts.yml"
}

setup_project_with_alice() {
    joy init --name "Forge Alias" --acronym FA
    joy auth init --passphrase "$FOUNDER_PASSPHRASE"
    local out
    out=$(joy project member add alice@example.com --passphrase "$FOUNDER_PASSPHRASE")
    local otp
    otp=$(extract_otp "$out")
    [ -n "$otp" ]
    # alice enrolls normally, under her primary address
    joy auth --otp "$otp" --user alice@example.com --passphrase "$ALICE_PASSPHRASE"
    # a GitHub remote makes joy-github the responsible plugin
    git remote add origin git@github.com:example/forge-alias.git
}

@test "a member behind a github alias keeps working via the forge plugin" {
    setup_project_with_alice
    install_gh_stub

    # alice's own clone: it carries GitHub's privacy alias (gh auth
    # setup-git) and knows nothing about who acts here yet
    forget_this_device
    git config user.email "777+alice-login@users.noreply.github.com"

    # login resolves through the connector chain: alias -> the GitHub
    # connector -> gh's token (stub) -> the forge (fake) ->
    # alice@example.com -> member
    run joy auth --passphrase "$ALICE_PASSPHRASE"
    [ "$status" -eq 0 ]

    # a real write passes the guard as alice
    run joy add idea "written behind the alias"
    [ "$status" -eq 0 ]

    # the audit trail carries the MEMBER, never the alias
    run grep -rl "users.noreply.github.com" .joy/items
    [ "$status" -ne 0 ]
    grep -q "created_by: alice@example.com" .joy/items/*.yaml
}

@test "without a responsible plugin the alias stays a stranger" {
    setup_project_with_alice
    # NO gh stub, NO gh config: the connector answers, but can vouch for
    # no addresses, so the resolution honestly fails like before.
    export GH_CONFIG_DIR="$TEST_DIR/empty-gh-config"
    forget_this_device
    git config user.email "777+alice-login@users.noreply.github.com"

    # the clone offers the alias, and nothing can place it
    run joy auth --passphrase "$ALICE_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not a registered project member"* ]]

    # so nobody acts here, and the write is refused as well. The refused
    # login left no session and no pin behind, and git config names
    # nobody since J11, so there is exactly ONE sentence a write can
    # answer with here: name yourself. Asserting it by name is what
    # keeps a regression in that sentence from passing as a refusal.
    run joy add idea "should be refused"
    [ "$status" -ne 0 ]
    [[ "$output" == *"this project does not know who you are, pick your member"* ]]
}

@test "joy init refuses a forge alias as founder identity" {
    # capture guard: the alias must never become a member key; the
    # refusal comes BEFORE anything is written
    git remote add origin git@github.com:example/fresh.git
    git config user.email "777+alice-login@users.noreply.github.com"
    run joy init --name "Alias Init" --acronym AL
    [ "$status" -ne 0 ]
    [[ "$output" == *"forge alias address"* ]]
    [ ! -d .joy ]
}

@test "a local-only project never consults any plugin" {
    joy init --name "Local Only" --acronym LO
    joy auth init --passphrase "$FOUNDER_PASSPHRASE"
    # no remotes at all; a stranger address fails exactly like always
    forget_this_device
    git config user.email "stranger@example.com"
    run joy auth --passphrase "$FOUNDER_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not a registered project member"* ]]

    # nothing was pinned and no session was opened, so the write asks the
    # caller to name themselves, in those words
    run joy add idea "stranger writes"
    [ "$status" -ne 0 ]
    [[ "$output" == *"this project does not know who you are, pick your member"* ]]
}

@test "a legacy alias member key resolves back to the actor (direction two)" {
    # Legacy shape: the project was FOUNDED under the alias while the
    # repo had no remote — no plugin was responsible, so nothing could
    # judge the address, exactly how such projects came to exist. The
    # member key in project.yaml IS the alias.
    git config user.email "777+alice-login@users.noreply.github.com"
    joy init --name "Legacy Alias" --acronym LA
    joy auth init --passphrase "$FOUNDER_PASSPHRASE"
    grep -q "777+alice-login@users.noreply.github.com" .joy/project.yaml

    # Later the repo gets its GitHub remote and the person's clone uses
    # the PRIMARY address. Direction two: the plugin ATTRIBUTES the alias
    # member key (pure resolve) and matches it to the signed-in actor.
    git remote add origin git@github.com:example/legacy-alias.git
    install_gh_stub
    forget_this_device
    git config user.email alice@example.com

    run joy auth --passphrase "$FOUNDER_PASSPHRASE"
    [ "$status" -eq 0 ]
    run joy add idea "acting as primary against an alias member key"
    [ "$status" -eq 0 ]
}

# The tea STUB: the Gitea forge boundary, same shape as the gh one. tea
# hands out a TOKEN through its credential helper (`tea login helper
# get`, the command D2.4 names), and the fake API answers the read the
# connector makes with it.
install_tea_stub() {
    start_fake_forge
    point_forge_at_fake codeberg.org gitea
    printf 'alice@example.com' > "$FAKE_FORGE_DIR/email"
    STUB_DIR="$TEST_DIR/stub-bin"
    mkdir -p "$STUB_DIR"
    cat > "$STUB_DIR/tea" <<'STUB'
#!/bin/sh
case "$*" in
"login helper get")
    cat >/dev/null
    echo "username=alice-login"
    echo "password=gta_alias-test-token"
    ;;
*) exit 1 ;;
esac
STUB
    chmod +x "$STUB_DIR/tea"
    export PATH="$STUB_DIR:$PATH"
    # tea's config names the signed-in login and its instance, offline
    export TEA_CONFIG_DIR="$TEST_DIR/tea-config"
    mkdir -p "$TEA_CONFIG_DIR"
    printf 'logins:\n- name: codeberg\n  url: https://codeberg.org/\n  user: alice-login\n' \
        > "$TEA_CONFIG_DIR/config.yml"
}

@test "a member behind a codeberg alias keeps working via the gitea plugin" {
    setup_project_with_alice
    # this project lives on Codeberg, not GitHub
    git remote remove origin
    git remote add origin git@codeberg.org:example/forge-alias.git
    install_tea_stub

    # the clone flips to Gitea's private-email alias
    forget_this_device
    git config user.email "alice-login@noreply.codeberg.org"

    run joy auth --passphrase "$ALICE_PASSPHRASE"
    [ "$status" -eq 0 ]

    run joy add idea "written behind the codeberg alias"
    [ "$status" -eq 0 ]

    run grep -rl "noreply.codeberg.org" .joy/items
    [ "$status" -ne 0 ]
    grep -q "created_by: alice@example.com" .joy/items/*.yaml
}

# The same person, twice in the project: enrolled under one address and
# still invited under another (JOY-0259-44, reported by an operator whose
# repo refused him). The forge vouches for BOTH, pending one FIRST, so
# order cannot be what decides who the caller is.
# bats test_tags=smoke
@test "an enrolled member wins over an open invitation for the same person" {
    setup_project_with_alice
    # the founder speaks the invitation (alice holds no manage), which
    # means naming the founder: alice is who this device acts as now
    act_as test@example.com "$FOUNDER_PASSPHRASE"
    joy project member add alice-second@example.com --passphrase "$FOUNDER_PASSPHRASE"
    install_gh_stub
    # the forge vouches for BOTH addresses, pending one first
    printf 'alice-second@example.com,alice@example.com' > "$FAKE_FORGE_DIR/email"
    forget_this_device
    git config user.email "777+alice-login@users.noreply.github.com"

    # the enrolled slot answers, so login and write go through
    run joy auth --passphrase "$ALICE_PASSPHRASE"
    [ "$status" -eq 0 ]
    run joy add idea "the enrolled entry wins"
    [ "$status" -eq 0 ]

    # attributed to the enrolled member, never to the pending slot
    grep -q "created_by: alice@example.com" .joy/items/*.yaml
    run grep -rl "alice-second@example.com" .joy/items
    [ "$status" -ne 0 ]
}

# GitHub's older privacy address carries no numeric id (JOY-0254-3C).
@test "the legacy alias form without a numeric id resolves too" {
    setup_project_with_alice
    install_gh_stub
    forget_this_device
    git config user.email "alice-login@users.noreply.github.com"

    run joy auth --passphrase "$ALICE_PASSPHRASE"
    [ "$status" -eq 0 ]
    run joy add idea "legacy alias form"
    [ "$status" -eq 0 ]

    grep -q "created_by: alice@example.com" .joy/items/*.yaml
    run grep -rl "users.noreply.github.com" .joy/items
    [ "$status" -ne 0 ]
}

# No instance lives in the code (JOY-025C-A7): the host comes from gh's
# own config, and the alias is matched by SHAPE, so an Enterprise Server
# on any domain works.
@test "an enterprise host from gh's own config is claimed" {
    setup_project_with_alice
    install_gh_stub
    printf 'github.com:\n    user: alice-login\nghe.example.com:\n    user: alice-login\n' \
        > "$GH_CONFIG_DIR/hosts.yml"
    git remote remove origin
    git remote add origin git@ghe.example.com:example/forge-alias.git
    forget_this_device
    git config user.email "777+alice-login@users.noreply.ghe.example.com"

    run joy auth --passphrase "$ALICE_PASSPHRASE"
    [ "$status" -eq 0 ]
    run joy add idea "enterprise host"
    [ "$status" -eq 0 ]
    grep -q "created_by: alice@example.com" .joy/items/*.yaml
}

# ...and a lookalike host nobody is signed in to is NOT claimed, so the
# address stays a stranger (JOY-025C-A7, the other half).
@test "a lookalike host is not claimed by the github plugin" {
    setup_project_with_alice
    install_gh_stub
    git remote remove origin
    git remote add origin git@github.com.evil.example:example/forge-alias.git
    forget_this_device
    git config user.email "777+alice-login@users.noreply.github.com.evil.example"

    run joy auth --passphrase "$ALICE_PASSPHRASE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not a registered project member"* ]]

    # and the write after it asks the caller to name themselves, in those
    # words: the lookalike host placed nobody, so nobody acts here
    run joy add idea "should be refused"
    [ "$status" -ne 0 ]
    [[ "$output" == *"this project does not know who you are, pick your member"* ]]
}

# The glab STUB: the GitLab forge boundary, same shape as the gh twin.
install_glab_stub() {
    start_fake_forge
    point_forge_at_fake gitlab.com gitlab
    printf 'alice@example.com' > "$FAKE_FORGE_DIR/email"
    STUB_DIR="$TEST_DIR/stub-bin"
    mkdir -p "$STUB_DIR"
    # glab hands out a TOKEN through its credential helper, the command
    # D2.4 names; `glab auth token` does not exist.
    cat > "$STUB_DIR/glab" <<'STUB'
#!/bin/sh
case "$*" in
"auth credential-helper get")
    cat >/dev/null
    echo "username=oauth2"
    echo "password=glpat_alias-test-token"
    ;;
*) exit 1 ;;
esac
STUB
    chmod +x "$STUB_DIR/glab"
    export PATH="$STUB_DIR:$PATH"
    export GLAB_CONFIG_DIR="$TEST_DIR/glab-config"
    mkdir -p "$GLAB_CONFIG_DIR"
    printf 'hosts:\n  gitlab.com:\n    user: alice-login\n' > "$GLAB_CONFIG_DIR/config.yml"
}

# The second proof of the same contract (JOY-0255-B3); GitLab's alias
# carries a dash where GitHub's carries a plus.
@test "a member behind a gitlab alias keeps working via the gitlab plugin" {
    setup_project_with_alice
    git remote remove origin
    git remote add origin git@gitlab.com:example/forge-alias.git
    install_glab_stub
    forget_this_device
    git config user.email "4711-alice-login@users.noreply.gitlab.com"

    run joy auth --passphrase "$ALICE_PASSPHRASE"
    [ "$status" -eq 0 ]
    run joy add idea "written behind the gitlab alias"
    [ "$status" -eq 0 ]

    run grep -rl "users.noreply.gitlab.com" .joy/items
    [ "$status" -ne 0 ]
    grep -q "created_by: alice@example.com" .joy/items/*.yaml
}

# The session is stored under the member key, not under whatever the
# caller was holding (JOY-0253-8A): --user must carry through to the
# lookup, or status reports unauthenticated right after a good login.
# The alias in the clone's git config is the prefill `--user` overrides,
# which is the one job git config has left since J11 (D3.9).
@test "joy auth --user carries through to the session lookup" {
    setup_project_with_alice
    install_gh_stub
    forget_this_device
    git config user.email "777+alice-login@users.noreply.github.com"

    run joy auth --user alice@example.com --passphrase "$ALICE_PASSPHRASE"
    [ "$status" -eq 0 ]
    run joy auth status --json
    [ "$status" -eq 0 ]
    [[ "$output" == *'"authenticated":true'* ]]
    [[ "$output" == *'"member":"alice@example.com"'* ]]

    run joy add idea "written after --user"
    [ "$status" -eq 0 ]
    grep -q "created_by: alice@example.com" .joy/items/*.yaml
}
