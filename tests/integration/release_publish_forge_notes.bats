#!/usr/bin/env bats
#
# The release notes reach the forge (JOY-0248-AE): a release may already
# exist when publish runs, because a tag-triggered forge workflow made it
# or an earlier publish pushed and then failed. That pre-made release
# carries only the installer section, so v0.20.0 shipped with an empty
# changelog. The notes must land anyway, above what is already there, and
# a second run must not repeat them.
#
# gh is the forge boundary and the one marked STUB. It is STATEFUL: what
# an edit writes, the next view reports, so idempotence is observed the
# way the forge would show it.

load setup

install_gh_release_stub() {
    STUB_DIR="$TEST_DIR/stub-bin"
    mkdir -p "$STUB_DIR"
    GH_CALLS="$TEST_DIR/gh-calls"
    BODY_FILE="$TEST_DIR/gh-body"
    : > "$GH_CALLS"
    printf '%s' "${1-}" > "$BODY_FILE"      # what the forge already carries
    cat > "$STUB_DIR/gh" <<'STUB'
#!/bin/sh
printf '%s\n' "$*" >> "$GH_CALLS"
notes=""
prev=""
for a in "$@"; do
    [ "$prev" = "--notes" ] && notes="$a"
    prev="$a"
done
case "$1 $2" in
"--version "*|"--version") echo "gh version 2.0.0" ;;
"auth status") exit 0 ;;
"release view")
    [ -s "$BODY_FILE" ] || exit 1
    python3 -c "import json,sys;print(json.dumps({'url':'https://forge.example/r/releases/tag/x','body':open(sys.argv[1]).read()}))" "$BODY_FILE"
    ;;
"release edit")
    printf '%s' "$notes" > "$BODY_FILE"
    ;;
"release create")
    printf '%s' "$notes" > "$BODY_FILE"
    echo "https://forge.example/r/releases/tag/x"
    ;;
*) exit 0 ;;
esac
STUB
    chmod +x "$STUB_DIR/gh"
    export PATH="$STUB_DIR:$PATH"
    export GH_CALLS BODY_FILE
}

# A project with a real remote, so publish reaches the forge step instead
# of dying at git push.
setup_publishable_project() {
    BARE="$TEST_DIR/origin.git"
    git init --bare --quiet "$BARE"
    joy init --name "Test" >/dev/null
    git add -A
    git commit -m "init [no-item]" --quiet
    git remote add origin "$BARE"
    git push --quiet -u origin HEAD
    joy release record patch --description "Fixed the thing" </dev/null >/dev/null
}

@test "a release that does not exist yet is created with the notes" {
    setup_publishable_project
    install_gh_release_stub ""

    run -0 joy release publish --forge github
    grep -q "release create" "$GH_CALLS"
    grep -q "Fixed the thing" "$BODY_FILE"
}

@test "notes are prepended to a release the forge workflow already made" {
    setup_publishable_project
    install_gh_release_stub "## Install
run the installer"

    run -0 joy release publish --forge github
    # the existing release is edited, never created a second time
    grep -q "release edit" "$GH_CALLS"
    run -1 grep -q "release create" "$GH_CALLS"

    # and the changelog sits ABOVE the installer section the workflow wrote
    run -0 cat "$BODY_FILE"
    [[ "$output" == *"Fixed the thing"* ]]
    [[ "$output" == *"## Install"* ]]
    before=${output%%## Install*}
    [[ "$before" == *"Fixed the thing"* ]]
}

@test "a second publish leaves the notes alone" {
    setup_publishable_project
    install_gh_release_stub "## Install"

    run -0 joy release publish --forge github
    first=$(cat "$BODY_FILE")

    # publish again against the forge as it now stands
    : > "$GH_CALLS"
    run -0 joy release publish --forge github
    run -1 grep -q "release edit" "$GH_CALLS"
    [ "$(cat "$BODY_FILE")" = "$first" ]
}
