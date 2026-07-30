#!/usr/bin/env bash
# Static, local contract audit for the GitHub release workflow.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKFLOW="$ROOT_DIR/.github/workflows/release.yml"
CI_WORKFLOW="$ROOT_DIR/.github/workflows/ci.yml"
COMPOSE="$ROOT_DIR/docker-compose.yml"
RELEASE_ALL="$ROOT_DIR/scripts/release-all.sh"
PACKAGE_RELEASE="$ROOT_DIR/scripts/package-release.sh"
LOCAL_PUBLISHER="$ROOT_DIR/scripts/publish-release-local.sh"
RELEASE_PROVENANCE="$ROOT_DIR/scripts/release-provenance.sh"
RELEASE_PROVENANCE_TEST="$ROOT_DIR/scripts/release-provenance-test.sh"
GITHUB_GOVERNANCE="$ROOT_DIR/scripts/github-governance.sh"
LOCAL_DOCKERFILE="$ROOT_DIR/Dockerfile.release"
DOCKER_EMBEDDING_CACHE="$ROOT_DIR/scripts/prepare-docker-embedding-cache.sh"
RELEASE_READINESS="$ROOT_DIR/scripts/release-readiness.sh"
EXCELLENCE_SMOKE="$ROOT_DIR/scripts/excellence-smoke.sh"
CROSS_CONFIG="$ROOT_DIR/Cross.toml"
PASS=0

pass() {
    PASS=$((PASS + 1))
    printf '   ok %s\n' "$1"
}

require_file_literal() {
    local label="$1"
    local file="$2"
    local literal="$3"
    if rg -Fq -- "$literal" "$file"; then
        pass "$label"
    else
        printf '   FAIL %s: missing %s in %s\n' "$label" "$literal" "$file" >&2
        exit 1
    fi
}

require_literal() {
    require_file_literal "$1" "$WORKFLOW" "$2"
}

require_no_workflow_tag_trigger() {
    if rg -q '^  push:$' "$WORKFLOW"; then
        printf '   FAIL release workflow must not trigger on tag pushes\n' >&2
        exit 1
    fi
    pass "release workflow is manual-only"
}

require_manual_only_ci() {
    require_file_literal "manual CI fallback" "$CI_WORKFLOW" "workflow_dispatch:"
    if rg -q '^  (push|pull_request|schedule):' "$CI_WORKFLOW"; then
        printf '   FAIL CI workflow must not trigger automatically\n' >&2
        exit 1
    fi
    pass "CI workflow is manual-only"
}

require_shell_syntax() {
    local script="$1"
    if bash -n "$script"; then
        pass "shell syntax $(basename "$script")"
    else
        printf '   FAIL shell syntax: %s\n' "$script" >&2
        exit 1
    fi
}

require_command_success() {
    local label="$1"
    shift
    if "$@" >/dev/null; then
        pass "$label"
    else
        printf '   FAIL %s\n' "$label" >&2
        exit 1
    fi
}

printf '== Release workflow audit\n'
printf '   workflow=%s\n' "$WORKFLOW"

require_literal "manual release fallback" "workflow_dispatch:"
require_no_workflow_tag_trigger
require_manual_only_ci

for target in \
    x86_64-unknown-linux-gnu \
    aarch64-unknown-linux-gnu \
    aarch64-apple-darwin \
    x86_64-apple-darwin \
    x86_64-pc-windows-msvc; do
    require_literal "target $target" "target: $target"
done

require_literal "shared Unix packager" "bash scripts/package-release.sh"
require_literal "compile-time version" 'CAPTAIN_BUILD_VERSION: ${{ github.ref_name }}'
require_literal "Windows platform manifest" 'manifest-${{ matrix.target }}.json'
require_literal "Windows installer asset" '            install.ps1'
require_literal "aggregate manifest" 'dist/release-assets/manifest-*.json > dist/release-assets/manifest.json'
require_literal "installer asset" 'dist/releases/${{ github.ref_name }}/install.sh'
require_literal "publish waits for Docker" "needs: [cli, docker]"
require_literal "Docker waits for validated CLI artifacts" "needs: cli"
require_literal "workflow prepares the deterministic embedding artifact" 'target/${{ matrix.target }}/release-fast/captain embeddings install'
require_literal "workflow uploads the deterministic embedding artifact" "- name: Upload deterministic Docker embedding cache"
require_literal "Docker downloads validated Linux bundles" "pattern: captain-*-unknown-linux-gnu"
require_literal "Docker downloads the deterministic embedding artifact" "- name: Download deterministic Docker embedding cache"
require_literal "Docker restores the embedding cache at its build path" "path: dist/docker/fastembed-cache"
require_literal "Docker QEMU setup" "docker/setup-qemu-action@v3"
require_literal "Docker Buildx setup" "docker/setup-buildx-action@v3"
require_literal "Docker multi-architecture platforms" "platforms: linux/amd64,linux/arm64"
require_literal "Docker consumes the packaged release version" 'CAPTAIN_RELEASE_VERSION=${{ github.ref_name }}'
require_literal "Docker immutable release tag" 'ghcr.io/${{ steps.image.outputs.owner }}/captain-agent-os:${{ github.ref_name }}'
require_literal "manual prerelease choice" 'prerelease:'
require_literal "manual image channel choice" 'image_channel:'
require_literal "Docker selected channel tag" 'ghcr.io/${{ steps.image.outputs.owner }}/captain-agent-os:${{ inputs.image_channel }}'
require_literal "GitHub Release preserves prerelease state" 'prerelease: ${{ inputs.prerelease }}'
require_literal "GitHub Release does not mark prereleases latest" "make_latest: \${{ inputs.prerelease && 'false' || 'true' }}"
require_file_literal "Compose defaults to the public alpha channel" "$COMPOSE" 'image: ghcr.io/vivien83/captain-agent-os:${CAPTAIN_IMAGE_TAG:-alpha}'

for target in \
    x86_64-unknown-linux-gnu \
    aarch64-unknown-linux-gnu \
    aarch64-apple-darwin \
    x86_64-apple-darwin \
    x86_64-pc-windows-msvc; do
    require_file_literal "local target $target" "$RELEASE_ALL" "$target"
done

require_file_literal "local publisher validates five platforms" "$LOCAL_PUBLISHER" 'x86_64-pc-windows-msvc'
require_file_literal "local publisher targets the isolated public image package" "$LOCAL_PUBLISHER" 'IMAGE="${CAPTAIN_IMAGE:-ghcr.io/$OWNER_LOWER/captain-agent-os}"'
require_file_literal "local publisher builds one Docker architecture at a time" "$LOCAL_PUBLISHER" '--platform "linux/$architecture"'
if rg -Fq -- '--platform linux/amd64,linux/arm64' "$LOCAL_PUBLISHER"; then
    printf '   FAIL local publisher must not build Docker architectures concurrently\n' >&2
    exit 1
fi
pass "local publisher forbids concurrent Docker architecture builds"
require_file_literal "local publisher builds amd64 before arm64" "$LOCAL_PUBLISHER" 'amd64_digest="$(build_and_push_architecture amd64)"'
require_file_literal "local publisher builds arm64 after amd64" "$LOCAL_PUBLISHER" 'arm64_digest="$(build_and_push_architecture arm64)"'
require_file_literal "local publisher assembles the Docker index after both builds" "$LOCAL_PUBLISHER" 'docker buildx imagetools create'
require_file_literal "local publisher emits maximal BuildKit provenance" "$LOCAL_PUBLISHER" '--provenance=mode=max'
require_file_literal "local publisher checks host capacity between builds" "$LOCAL_PUBLISHER" 'release_host_checkpoint "after Docker linux/$architecture"'
require_file_literal "local publisher uses prebuilt release image" "$LOCAL_PUBLISHER" '--file Dockerfile.release'
require_file_literal "local publisher creates GitHub Release" "$LOCAL_PUBLISHER" 'gh release create'
require_file_literal "local publisher derives release channel" "$LOCAL_PUBLISHER" 'release_channel_for_version'
require_file_literal "local publisher marks prereleases" "$LOCAL_PUBLISHER" 'create_args+=(--prerelease)'
require_file_literal "local publisher uses reviewed notes" "$LOCAL_PUBLISHER" 'docs/releases/$VERSION.md'
require_file_literal "local publisher verifies remote image" "$LOCAL_PUBLISHER" 'docker buildx imagetools inspect'
require_file_literal "local publisher verifies amd64 image" "$LOCAL_PUBLISHER" 'index("amd64")'
require_file_literal "local publisher verifies arm64 image" "$LOCAL_PUBLISHER" 'index("arm64")'
require_file_literal "local publisher supports offline asset validation" "$LOCAL_PUBLISHER" 'CAPTAIN_VALIDATE_ONLY'
require_file_literal "local publisher validates bundle versions" "$LOCAL_PUBLISHER" 'embedded bundle version mismatch'
require_file_literal "local publisher validates Windows PE" "$LOCAL_PUBLISHER" 'Windows bundle does not contain a PE executable'
require_file_literal "local publisher stages deterministic embeddings" "$LOCAL_PUBLISHER" 'scripts/prepare-docker-embedding-cache.sh'
require_file_literal "local publisher generates host artifact provenance" "$LOCAL_PUBLISHER" 'scripts/release-provenance.sh'
require_file_literal "local publisher uploads the provenance statement" "$LOCAL_PUBLISHER" 'provenance.intoto.jsonl'
require_file_literal "local packager checks host capacity per target" "$RELEASE_ALL" 'release_host_checkpoint "after $target"'
require_file_literal "provenance uses in-toto Statement v1" "$RELEASE_PROVENANCE" 'https://in-toto.io/Statement/v1'
require_file_literal "provenance uses SLSA v1" "$RELEASE_PROVENANCE" 'https://slsa.dev/provenance/v1'
require_file_literal "provenance binds Cargo.lock" "$RELEASE_PROVENANCE" '"uri": "file:Cargo.lock"'
require_file_literal "provenance records sequential target builds" "$RELEASE_PROVENANCE" '"parallelTargetBuilds": false'
require_file_literal "Unix platform manifests bind source revision" "$PACKAGE_RELEASE" '"revision": "$SOURCE_REVISION"'
require_file_literal "Windows platform manifest binds source revision" "$RELEASE_ALL" '"revision": "$SOURCE_REVISION"'
require_file_literal "Unix platform manifests record the real build start" "$PACKAGE_RELEASE" '"build_started_at": "$BUILD_STARTED_AT"'
require_file_literal "Windows platform manifest records the real build start" "$RELEASE_ALL" '"build_started_at": "$build_started_at"'
require_file_literal "aggregate manifest rejects mixed source provenance" "$PACKAGE_RELEASE" 'platform manifests contain mixed source provenance'
require_file_literal "provenance test rejects mixed source revisions" "$RELEASE_PROVENANCE_TEST" 'mixed source revisions were accepted'
require_file_literal "branch protection requires review" "$GITHUB_GOVERNANCE" 'required_approving_review_count: 1'
require_file_literal "branch protection keeps hosted checks optional" "$GITHUB_GOVERNANCE" 'required_status_checks: null'
require_file_literal "personal-repository protection omits organization-only bypass allowances" "$GITHUB_GOVERNANCE" 'has("bypass_pull_request_allowances") | not'
require_file_literal "branch protection rejects force pushes" "$GITHUB_GOVERNANCE" 'allow_force_pushes: false'
require_file_literal "branch protection rejects branch deletion" "$GITHUB_GOVERNANCE" 'allow_deletions: false'
require_file_literal "cross propagates compile-time version" "$CROSS_CONFIG" 'passthrough = ["CAPTAIN_BUILD_VERSION"]'
require_file_literal "ARM Cross bounds the primary Ubuntu Ports probe" "$CROSS_CONFIG" 'curl -fsS --connect-timeout 5 --max-time 15 http://ports.ubuntu.com'
require_file_literal "ARM Cross has a registered Ubuntu Ports fallback" "$CROSS_CONFIG" 'http://mirrors.ocf.berkeley.edu/ubuntu-ports'
require_file_literal "x86 Cross skips the irrelevant Ubuntu ports mirror" "$CROSS_CONFIG" 'rm -f /etc/apt/sources.list.d/ports.list'
require_file_literal "local packager executes embedded versions" "$RELEASE_ALL" 'embedded binary version mismatch'
require_file_literal "local packager emulates Linux ARM64" "$RELEASE_ALL" 'qemu-aarch64-static'
require_file_literal "local packager forwards custom output roots" "$RELEASE_ALL" 'CAPTAIN_DIST_DIR="${CAPTAIN_DIST_DIR:-dist/releases}"'
require_file_literal "local packager honors the shared Cargo target root" "$RELEASE_ALL" 'TARGET_ROOT="${CARGO_TARGET_DIR:-target}"'
require_file_literal "local packager resolves the host binary from the shared target root" "$RELEASE_ALL" 'echo "$TARGET_ROOT/release/captain"'
require_file_literal "local packager opens current Docker Desktop app" "$RELEASE_ALL" 'open -a "Docker Desktop"'
require_file_literal "local packager retains legacy Docker app fallback" "$RELEASE_ALL" 'open -a Docker'
require_file_literal "macOS bundle signing fails closed" "$PACKAGE_RELEASE" 'failed to verify $PLATFORM release signature'
require_file_literal "Windows preflight requires LLVM" "$RELEASE_ALL" 'command -v llvm-ar'
require_file_literal "Windows preflight requires NASM" "$RELEASE_ALL" 'command -v nasm'
require_file_literal "Windows excludes Unix OpenSSL forcing" "$ROOT_DIR/crates/captain-channels/Cargo.toml" '[target.'\''cfg(not(target_os = "windows"))'\''.dependencies]'
require_file_literal "release image consumes amd64 bundle" "$LOCAL_DOCKERFILE" 'captain-x86_64-unknown-linux-gnu.tar.gz'
require_file_literal "release image consumes arm64 bundle" "$LOCAL_DOCKERFILE" 'captain-aarch64-unknown-linux-gnu.tar.gz'
require_file_literal "release image preserves runtime VERSION" "$LOCAL_DOCKERFILE" 'COPY --from=captain-binary /out/VERSION /usr/local/bin/VERSION'
require_file_literal "release image copies staged embeddings" "$LOCAL_DOCKERFILE" 'COPY dist/docker/fastembed-cache/'
require_file_literal "release image verifies staged embeddings" "$LOCAL_DOCKERFILE" 'sha256sum --check CAPTAIN-SHA256SUMS'
require_file_literal "embedding cache pins the model revision" "$DOCKER_EMBEDDING_CACHE" '5f1b8cd78bc4fb444dd171e59b18f3a3af89a079'
require_file_literal "embedding cache pins the model checksum" "$DOCKER_EMBEDDING_CACHE" 'bbd7b466f6d58e646fdc2bd5fd67b2f5e93c0b687011bd4548c420f7bd46f0c5'
require_file_literal "embedding cache output stays ignored" "$ROOT_DIR/.gitignore" '/dist/docker/'
require_file_literal "release readiness defaults to release Cargo" "$RELEASE_READINESS" 'CARGO_PROFILE="${CAPTAIN_RELEASE_CARGO_PROFILE:-release}"'
require_file_literal "release readiness versions release-profile tests" "$RELEASE_READINESS" 'run env CAPTAIN_BUILD_VERSION="$EXPECTED_CHANGELOG" cargo test --release "$@"'
require_file_literal "release readiness rejects unknown Cargo profiles" "$RELEASE_READINESS" 'CAPTAIN_RELEASE_CARGO_PROFILE must be dev or release'
require_file_literal "release readiness resolves the built candidate" "$RELEASE_READINESS" 'release_candidate_bin'
require_file_literal "release readiness isolates the candidate home" "$RELEASE_READINESS" 'captain-release-smoke.XXXXXX'
require_file_literal "release readiness provisions native MemPalace" "$RELEASE_READINESS" 'CAPTAIN_MEMPALACE_INSTALL=1 CAPTAIN_HOME="$home_dir"'
require_file_literal "release readiness requires process-tree discovery" "$RELEASE_READINESS" 'need_cmd pgrep'
require_file_literal "release readiness captures smoke descendants before the parent" "$RELEASE_READINESS" 'descendants="$(collect_process_descendants "$SMOKE_PID")"'
require_file_literal "release readiness terminates smoke descendants first" "$RELEASE_READINESS" 'signal_process_list TERM "$descendants"'
require_file_literal "release readiness verifies candidate HTTP version" "$RELEASE_READINESS" 'isolated release candidate health version mismatch'
require_file_literal "release smoke requires only the principal agent" "$EXCELLENCE_SMOKE" 'fresh release starts with only the captain agent'
require_file_literal "release smoke keeps header arguments nonempty on Bash 3.2" "$EXCELLENCE_SMOKE" 'AUTH_HEADER_ARGS=(-H "Accept: application/json")'
require_file_literal "release smoke preserves absolute artifact roots" "$EXCELLENCE_SMOKE" '/*) doc_path="$WORKDIR/excellence-smoke.md"'
require_shell_syntax "$ROOT_DIR/scripts/package-release.sh"
require_shell_syntax "$RELEASE_ALL"
require_shell_syntax "$LOCAL_PUBLISHER"
require_shell_syntax "$RELEASE_PROVENANCE"
require_shell_syntax "$RELEASE_PROVENANCE_TEST"
require_shell_syntax "$GITHUB_GOVERNANCE"
require_shell_syntax "$DOCKER_EMBEDDING_CACHE"
require_shell_syntax "$RELEASE_READINESS"
require_shell_syntax "$EXCELLENCE_SMOKE"
require_command_success "release readiness help exits cleanly" "$RELEASE_READINESS" --help
CAPTAIN_RELEASE_POLICY_TEST=1 "$LOCAL_PUBLISHER" >/dev/null
pass "local release channel policy"
require_command_success "release provenance contract" "$RELEASE_PROVENANCE_TEST"
require_command_success "GitHub governance policy" "$GITHUB_GOVERNANCE" --policy-test

printf '\nRelease workflow audit passed: %s checks.\n' "$PASS"
