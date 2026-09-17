#!/usr/bin/env bats
#
# The release notes reach the forge (JOY-0248-AE): a release may already
# exist when publish runs, because a tag-triggered forge workflow made it
# or an earlier publish pushed and then failed. That pre-made release
# carries only the installer section, so v0.20.0 shipped with an empty
# changelog. The notes must land anyway, above what is already there, and
# a second run must not repeat them.
#
# Since JOY-0298-E4 the release verb speaks REST through the connector's
# own HTTP client (design D2.8), so the forge boundary is the fake API
# and gh is only a source of a TOKEN (decision 19). The fake is
# STATEFUL: what a PATCH writes, the next GET reports, so idempotence is
# observed the way the forge would show it.

load setup
load forge_fake

# A project with a real remote, so publish reaches the forge step
# instead of dying at git push.
#
# The remote URL is a GitHub one, because that is what tells the
# connector which repository the release belongs to; git's own
# `pushurl` sends the push to a local bare repository, so the product
# path is unchanged and nothing leaves the machine.
setup_publishable_project() {
    BARE="$TEST_DIR/origin.git"
    git init --bare --quiet "$BARE"
    joy init --name "Test" >/dev/null
    git add -A
    git commit -m "init [no-item]" --quiet
    git remote add origin "https://github.com/example/demo.git"
    git config remote.origin.pushurl "$BARE"
    git push --quiet -u origin HEAD
    joy release record patch --description "Fixed the thing" </dev/null >/dev/null
}

# The forge boundary: the fake API for the REST calls, gh for the token.
install_forge_stub() {
    start_fake_forge
    point_forge_at_fake github.com github
    printf '%s' "${1-}" > "$FAKE_FORGE_DIR/release_body"
    install_gh_token_stub "gho_release-test-token"
}

teardown_fake() {
    stop_fake_forge
}

@test "a release that does not exist yet is created with the notes" {
    setup_publishable_project
    install_forge_stub ""

    run -0 joy release publish --forge github
    grep -q "POST /repos/example/demo/releases" "$FAKE_FORGE_DIR/calls"
    grep -q "Fixed the thing" "$FAKE_FORGE_DIR/release_body"
    teardown_fake
}

@test "notes are prepended to a release the forge workflow already made" {
    setup_publishable_project
    install_forge_stub "## Install
run the installer"

    run -0 joy release publish --forge github
    # the existing release is completed, never created a second time
    grep -q "PATCH /repos/example/demo/releases/1" "$FAKE_FORGE_DIR/calls"
    run -1 grep -q "POST /repos/example/demo/releases " "$FAKE_FORGE_DIR/calls"

    # and the changelog sits ABOVE the installer section the workflow wrote
    run -0 cat "$FAKE_FORGE_DIR/release_body"
    [[ "$output" == *"Fixed the thing"* ]]
    [[ "$output" == *"## Install"* ]]
    before=${output%%## Install*}
    [[ "$before" == *"Fixed the thing"* ]]
    teardown_fake
}

@test "a second publish leaves the notes alone" {
    setup_publishable_project
    install_forge_stub "## Install"

    run -0 joy release publish --forge github
    first=$(cat "$FAKE_FORGE_DIR/release_body")

    # publish again against the forge as it now stands
    : > "$FAKE_FORGE_DIR/calls"
    run -0 joy release publish --forge github
    run -1 grep -q "PATCH " "$FAKE_FORGE_DIR/calls"
    [ "$(cat "$FAKE_FORGE_DIR/release_body")" = "$first" ]
    teardown_fake
}

@test "publishing needs neither curl nor gh on the API path" {
    setup_publishable_project
    start_fake_forge
    point_forge_at_fake github.com github
    : > "$FAKE_FORGE_DIR/release_body"
    # The machine of J2's acceptance: no gh and no curl exist at all.
    # The PATH is built from nothing but joy (with the connector beside
    # it) and the handful of tools this case itself needs, so a gh or a
    # curl on the developer's machine cannot answer for the connector.
    mkdir -p "$TEST_DIR/min-bin"
    for tool in git grep; do
        ln -sf "$(command -v "$tool")" "$TEST_DIR/min-bin/$tool"
    done
    SAVED_PATH="$PATH"
    PATH="$(dirname "$JOY_BIN"):$TEST_DIR/min-bin"
    export PATH SAVED_PATH
    run -1 command -v gh
    run -1 command -v curl
    # the token is the forge's own variable, which is what a person or a
    # CI runner exports
    export GH_TOKEN="gho_only-in-the-environment"

    run -0 joy release publish --forge github
    grep -q "POST /repos/example/demo/releases" "$FAKE_FORGE_DIR/calls"
    grep -q "Bearer gho_only-in-the-environment" "$FAKE_FORGE_DIR/authorization"
    teardown_fake
}
