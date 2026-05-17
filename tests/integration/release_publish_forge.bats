#!/usr/bin/env bats
# JOY-0197-EB: joy release publish auto-detects forge from configured remotes.
# Covers the resolution layer; calling out to gh is out of scope here.

load setup

setup_publish_repo() {
    # Skip joy auth init on purpose: with no registered members the
    # guard runs permissive and lets `joy release record` create the
    # tag without a Manage capability. release_record_empty.bats uses
    # the same shape. We only care about the publish-time forge
    # resolution here, not about gating.
    joy init --name "Test" >/dev/null
    git add -A
    git commit -m "init [no-item]" --quiet
    joy release record patch </dev/null >/dev/null
}

@test "publish without forge and without remotes fails with actionable error" {
    setup_publish_repo
    run joy release publish
    [ "$status" -ne 0 ]
    [[ "$output" == *"no supported forge detected"* ]]
    [[ "$output" == *"no git remotes are configured"* ]]
}

@test "publish without forge but with only an unsupported remote fails with remote listed" {
    setup_publish_repo
    git remote add origin https://example.com/fake.git
    run joy release publish
    [ "$status" -ne 0 ]
    [[ "$output" == *"no supported forge detected"* ]]
    [[ "$output" == *"configured remotes: origin"* ]]
}

@test "publish --forge none skips forge release and exits 0 after push (or push failure surfaces clearly)" {
    setup_publish_repo
    # --forge none must short-circuit the forge decision before push.
    # Without a real remote the push step still fails, but the error
    # must come from git push, not from forge resolution.
    run joy release publish --forge none
    [ "$status" -ne 0 ]
    [[ "$output" != *"no supported forge detected"* ]]
    [[ "$output" != *"unsupported forge"* ]]
}

@test "publish auto-detects github from a single supported remote" {
    setup_publish_repo
    git remote add origin git@github.com:fake/repo.git
    # The push will fail (no real remote) but the auto-detect note
    # must appear before that and forge resolution must succeed.
    run joy release publish
    [ "$status" -ne 0 ]
    [[ "$output" == *"auto-detected forge 'github' from remote 'origin'"* ]]
    [[ "$output" != *"no supported forge detected"* ]]
}

@test "publish on non-TTY with multiple supported forges fails with --forge hint" {
    setup_publish_repo
    git remote add origin git@github.com:fake/repo.git
    git remote add mirror git@github.com:fake/mirror.git
    # Two GitHub remotes still dedupe to one supported forge. Use a
    # distinct (currently-unsupported) host plus a "fake support" by
    # using two different recognized hosts: github + gitea.
    git remote remove mirror
    git remote add gitea_mirror https://gitea.example.com/fake/repo.git
    # Today only github is in SUPPORTED_FORGES so this still dedupes
    # to one. Skip the multi-pick path until gitea is supported.
    skip "multi-supported-forge path needs a second supported backend"
}

@test "publish --forge with unsupported value fails before push" {
    setup_publish_repo
    git remote add origin git@github.com:fake/repo.git
    run joy release publish --forge gitlab
    [ "$status" -ne 0 ]
    [[ "$output" == *"unsupported forge 'gitlab'"* ]]
    [[ "$output" == *"--forge"* ]]
    # Push must not have run -- the resolution step rejects upfront.
    [[ "$output" != *"Pushing to"* ]]
}
