#!/usr/bin/env bats
#
# Forge-plugin alias resolution, end to end (epic JOY-0251-AA,
# JOY-0253-8A / JOY-0254-3C, reported as JP-00BF-94): a member enrolled
# under their PRIMARY address keeps working when the clone's git config
# carries GitHub's noreply alias. joy-core resolves via the joy-github
# plugin; the plugin consults gh. The forge boundary (gh, the one thing
# tests cannot have for real) is a MARKED STUB; everything else is the
# real product path: real joy, real joy-github, real project.

load setup

FOUNDER_PASSPHRASE="correct horse battery staple extra words"
ALICE_PASSPHRASE="alpha bravo charlie delta echo foxtrot"

extract_otp() {
    echo "$1" | sed -n 's/^[[:space:]]*One-time password:[[:space:]]*\([A-Za-z0-9-]*\).*$/\1/p' | head -1
}

# The gh STUB: answers exactly the two API reads joy-github performs.
# This is the forge boundary; nothing else is faked.
install_gh_stub() {
    STUB_DIR="$TEST_DIR/stub-bin"
    mkdir -p "$STUB_DIR"
    cat > "$STUB_DIR/gh" <<'EOF'
#!/bin/sh
case "$1 $2" in
"api user/emails") echo '[{"email":"alice@example.com","verified":true}]' ;;
"api user") echo '{"email":null}' ;;
*) exit 1 ;;
esac
EOF
    chmod +x "$STUB_DIR/gh"
    export PATH="$STUB_DIR:$PATH"
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
    git config user.email alice@example.com
    joy auth --otp "$otp" --passphrase "$ALICE_PASSPHRASE"
    # a GitHub remote makes joy-github the responsible plugin
    git remote add origin git@github.com:example/forge-alias.git
}

@test "a member behind a github alias keeps working via the forge plugin" {
    setup_project_with_alice
    install_gh_stub

    # the clone flips to GitHub's privacy alias (gh auth setup-git)
    git config user.email "777+alice-login@users.noreply.github.com"

    # login resolves through the plugin chain: alias -> joy-github ->
    # gh (stub) -> alice@example.com -> member
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
    # NO gh stub, NO gh config: joy-github answers, but can vouch for no
    # addresses, so the resolution honestly fails like before.
    export GH_CONFIG_DIR="$TEST_DIR/empty-gh-config"
    git config user.email "777+alice-login@users.noreply.github.com"
    run joy add idea "should be refused"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not a registered project member"* ]] || [[ "$output" == *"must authenticate"* ]]
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
    git config user.email "stranger@example.com"
    run joy add idea "stranger writes"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not a registered project member"* ]] || [[ "$output" == *"must authenticate"* ]]
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
    git config user.email alice@example.com

    run joy auth --passphrase "$FOUNDER_PASSPHRASE"
    [ "$status" -eq 0 ]
    run joy add idea "acting as primary against an alias member key"
    [ "$status" -eq 0 ]
}

# The tea STUB: the Gitea forge boundary, same shape as the gh stub.
install_tea_stub() {
    STUB_DIR="$TEST_DIR/stub-bin"
    mkdir -p "$STUB_DIR"
    cat > "$STUB_DIR/tea" <<'STUB'
#!/bin/sh
case "$*" in
"api get user/emails") echo '[{"email":"alice@example.com","verified":true}]' ;;
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
    git config user.email "alice-login@noreply.codeberg.org"

    run joy auth --passphrase "$ALICE_PASSPHRASE"
    [ "$status" -eq 0 ]

    run joy add idea "written behind the codeberg alias"
    [ "$status" -eq 0 ]

    run grep -rl "noreply.codeberg.org" .joy/items
    [ "$status" -ne 0 ]
    grep -q "created_by: alice@example.com" .joy/items/*.yaml
}
