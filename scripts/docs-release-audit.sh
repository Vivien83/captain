#!/usr/bin/env bash
# Reproducible audit for release-facing docs claims.

set -u

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="${TMPDIR:-/tmp}/captain-docs-release-audit.$$"
PASS=0
FAIL=0

mkdir -p "$TMP_DIR" || exit 1
trap 'rm -rf "$TMP_DIR"' EXIT

DOC_FILES=(
  README.md
  README.fr.md
  README.es.md
  README.zh.md
  CHANGELOG.md
  CONTRIBUTING.md
  docs/api-reference.md
  docs/architecture.md
  docs/providers.md
  docs/cli-reference.md
  docs/channel-adapters.md
  docs/configuration.md
  docs/release-provenance.md
  docs/repository-governance.md
  docs/captain-tools/channel.md
  docs/captain-tools/memory.md
  docs/captain-tools/skill.md
  docs/captain-tools/runtime-changelog.md
  docs/captain-tools/config-secret.md
  docs/INDEX.md
  docs/DOCS_STATUS.md
  docs/DEPLOY.md
  docs/getting-started.md
  docs/troubleshooting.md
  docs/deployment/github-vps-install.md
  docs/releases/v0.1.0-alpha.12.md
  docs/releases/v0.1.0-alpha.11.md
  docs/releases/v0.1.0-alpha.10.md
  docs/releases/v0.1.0-alpha.9.md
  docs/releases/v0.1.0-alpha.8.md
  docs/releases/v0.1.0-alpha.7.md
  docs/releases/v0.1.0-alpha.6.md
  docs/releases/v0.1.0-alpha.5.md
  docs/releases/v0.1.0-alpha.4.md
  docs/releases/v0.1.0-alpha.3.md
  docs/releases/v0.1.0-alpha.2.md
  docs/releases/v0.1.0-alpha.1.md
)

pass() {
  PASS=$((PASS + 1))
  printf '   ok %s\n' "$1"
}

fail() {
  FAIL=$((FAIL + 1))
  printf '   FAIL %s\n' "$1" >&2
}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    fail "missing required command: $1"
    finish
  fi
}

finish() {
  printf '\n========================================\n'
  if [ "$FAIL" -eq 0 ]; then
    printf 'Docs release audit passed: %s checks.\n' "$PASS"
    exit 0
  fi
  printf 'Docs release audit failed: %s failed, %s passed.\n' "$FAIL" "$PASS"
  exit 1
}

show_findings() {
  local file="$1"
  sed -n '1,40p' "$file"
}

scan_banned() {
  local label="$1"
  local pattern="$2"
  local out="$TMP_DIR/$label.txt"
  rg -n "$pattern" "${DOC_FILES[@]}" >"$out" || true
  if [ -s "$out" ]; then
    fail "$label"
    show_findings "$out"
  else
    pass "$label"
  fi
}

require_contains() {
  local label="$1"
  local file="$2"
  local pattern="$3"
  if grep -Fq "$pattern" "$file"; then
    pass "$label"
  else
    fail "$label"
  fi
}

require_not_contains() {
  local label="$1"
  local file="$2"
  local pattern="$3"
  if grep -Fq "$pattern" "$file"; then
    fail "$label"
  else
    pass "$label"
  fi
}

cd "$ROOT_DIR" || exit 1
SITE_PRESENT=0
if [ -f site/index.html ]; then
  SITE_PRESENT=1
  DOC_FILES+=(
    docs/deployment/launch-site.md
    site/index.html
    site/assets/site.js
    site/assets/terminal-demo.js
  )
fi
need_cmd rg
need_cmd sed

printf '== Docs release audit\n'
printf '   root=%s\n' "$ROOT_DIR"

scan_banned \
  "no stale active channel-count claims" \
  '40 channel adapters|40 channels|42 channels|All 40|Forty channels'

scan_banned \
  "no active marketplace or ClawHub claims" \
  'Captain Marketplace|Skill marketplace|ClawHub client|Install from ClawHub|Search ClawHub|Browse ClawHub'

scan_banned \
  "no stale global test-count claims" \
  '1,744\+ tests|967 tests|2,725 tests'

scan_banned \
  "no stale README tool-model-skill counts" \
  '190\+ built-in tools|217 models|65 bundled skills|plus de 190 outils|217 modèles|65 skills packagés|más de 190 herramientas|217 modelos|65 skills incluidas|190 多个内置工具|217 个模型|65 个内置 skill'

scan_banned \
  "no active non-core channel examples" \
  'Add Matrix channel adapter|Telegram, Discord, Slack|Slack, WhatsApp, Signal, Matrix, Email'

require_contains \
  "current release candidate has an agent-facing changelog" \
  docs/captain-tools/runtime-changelog.md \
  "### 0.1.0-alpha.12"
require_contains \
  "release readiness expects the current candidate" \
  scripts/release-readiness.sh \
  '0.1.0-alpha.12'
require_contains \
  "excellence smoke expects the current candidate" \
  scripts/excellence-smoke.sh \
  '0.1.0-alpha.12'
require_contains \
  "public changelog exposes the release candidate" \
  CHANGELOG.md \
  '## [0.1.0-alpha.12] - 2026-08-09'
require_contains \
  "published release notes exist" \
  docs/releases/v0.1.0-alpha.12.md \
  '# Captain 0.1.0-alpha.12'
require_contains \
  "historical alpha.11 notes remain available" \
  docs/releases/v0.1.0-alpha.11.md \
  '# Captain 0.1.0-alpha.11'
require_contains \
  "historical alpha.10 notes remain available" \
  docs/releases/v0.1.0-alpha.10.md \
  '# Captain 0.1.0-alpha.10'
require_contains \
  "historical alpha.9 notes remain available" \
  docs/releases/v0.1.0-alpha.9.md \
  '# Captain 0.1.0-alpha.9'
require_contains \
  "last published alpha.3 notes pin the public source commit" \
  docs/releases/v0.1.0-alpha.3.md \
  '13b8aca8d6d5f842cc93a23b9f03caf972f01bf1'
require_contains \
  "last published alpha.3 notes pin the multi-arch digest" \
  docs/releases/v0.1.0-alpha.3.md \
  'sha256:f7ff11969ed8b75b31c15dbc610fd785f4983f17e322f0501eea627df08ea4a2'
require_contains \
  "last published alpha.4 notes pin the public source commit" \
  docs/releases/v0.1.0-alpha.4.md \
  'a58bb3bcf5563beaee6b10d7672284c4c1ab9aa4'
require_contains \
  "last published alpha.4 notes pin the multi-arch digest" \
  docs/releases/v0.1.0-alpha.4.md \
  'sha256:4bdf0e224d95f7a5cd14360d2e2abb9c3bb7dfbe757fdedddab4c0246ec8aa93'
require_contains \
  "published alpha.5 notes pin the public source commit" \
  docs/releases/v0.1.0-alpha.5.md \
  '6d159dbf7597a0d7710e3362d4422e557c51ee10'
require_contains \
  "published alpha.5 notes pin the multi-arch digest" \
  docs/releases/v0.1.0-alpha.5.md \
  'sha256:412921cd69726152235bc08614d185686ebe8a34490ee11b42a94a79e0ddc873'
require_not_contains \
  "alpha.5 notes do not copy the alpha.4 source commit" \
  docs/releases/v0.1.0-alpha.5.md \
  'a58bb3bcf5563beaee6b10d7672284c4c1ab9aa4'
require_not_contains \
  "alpha.5 notes do not copy the alpha.4 OCI digest" \
  docs/releases/v0.1.0-alpha.5.md \
  'sha256:4bdf0e224d95f7a5cd14360d2e2abb9c3bb7dfbe757fdedddab4c0246ec8aa93'
require_not_contains \
  "alpha.6 notes do not copy the alpha.5 source commit" \
  docs/releases/v0.1.0-alpha.6.md \
  '6d159dbf7597a0d7710e3362d4422e557c51ee10'
require_not_contains \
  "alpha.6 notes do not copy the alpha.5 OCI digest" \
  docs/releases/v0.1.0-alpha.6.md \
  'sha256:412921cd69726152235bc08614d185686ebe8a34490ee11b42a94a79e0ddc873'
require_contains \
  "published alpha.6 notes pin the public source commit" \
  docs/releases/v0.1.0-alpha.6.md \
  '797d093b44a93850b40f058691931c25f1701900'
require_contains \
  "published alpha.6 notes pin the multi-arch digest" \
  docs/releases/v0.1.0-alpha.6.md \
  'sha256:1054e053d7f20664c4098db04d653e44b261d6cc4bac092a5fbc10a9e76c9318'
require_not_contains \
  "alpha.7 notes do not copy the alpha.6 source commit" \
  docs/releases/v0.1.0-alpha.7.md \
  '797d093b44a93850b40f058691931c25f1701900'
require_not_contains \
  "alpha.7 notes do not copy the alpha.6 OCI digest" \
  docs/releases/v0.1.0-alpha.7.md \
  'sha256:1054e053d7f20664c4098db04d653e44b261d6cc4bac092a5fbc10a9e76c9318'
require_contains \
  "published alpha.7 notes pin the public source commit" \
  docs/releases/v0.1.0-alpha.7.md \
  'dc2f64603eff708a8eab5735121cfc1a2d39386f'
require_contains \
  "published alpha.7 notes pin the multi-arch digest" \
  docs/releases/v0.1.0-alpha.7.md \
  'sha256:e49e1ad02d6a65742343aaf7abcd1c4fcfd277dab605d3d284830f03c7d42354'
require_not_contains \
  "alpha.8 notes do not copy the alpha.7 source commit" \
  docs/releases/v0.1.0-alpha.8.md \
  'dc2f64603eff708a8eab5735121cfc1a2d39386f'
require_not_contains \
  "alpha.8 notes do not copy the alpha.7 OCI digest" \
  docs/releases/v0.1.0-alpha.8.md \
  'sha256:e49e1ad02d6a65742343aaf7abcd1c4fcfd277dab605d3d284830f03c7d42354'
require_contains \
  "published alpha.8 notes pin the public source commit" \
  docs/releases/v0.1.0-alpha.8.md \
  'd82f120153b8e83e9be82df6748f928f8d4aa6b9'
require_contains \
  "published alpha.8 notes pin the multi-arch digest" \
  docs/releases/v0.1.0-alpha.8.md \
  'sha256:af32a605de0a019482ff3aadcee07179171630ccfb45c9b88fbcf135d2680230'
require_contains \
  "published alpha.8 notes record the zero-Actions proof" \
  docs/releases/v0.1.0-alpha.8.md \
  'GitHub Actions API returned zero runs'
require_not_contains \
  "alpha.9 notes do not copy the alpha.8 source commit" \
  docs/releases/v0.1.0-alpha.9.md \
  'd82f120153b8e83e9be82df6748f928f8d4aa6b9'
require_not_contains \
  "alpha.9 notes do not copy the alpha.8 OCI digest" \
  docs/releases/v0.1.0-alpha.9.md \
  'sha256:af32a605de0a019482ff3aadcee07179171630ccfb45c9b88fbcf135d2680230'
require_contains \
  "published alpha.9 notes pin the public source commit" \
  docs/releases/v0.1.0-alpha.9.md \
  '1248c5928dd4968b6ff7c62ef79a607fb8d94348'
require_contains \
  "published alpha.9 notes pin the multi-arch digest" \
  docs/releases/v0.1.0-alpha.9.md \
  'sha256:b043ec5637551c2e238be15c32033ca693ecc2f765a470ba721a5986709fd692'
require_contains \
  "published alpha.9 notes record the zero-Actions proof" \
  docs/releases/v0.1.0-alpha.9.md \
  'GitHub Actions API returned zero runs'
require_not_contains \
  "alpha.10 notes do not copy the alpha.9 source commit" \
  docs/releases/v0.1.0-alpha.10.md \
  '1248c5928dd4968b6ff7c62ef79a607fb8d94348'
require_not_contains \
  "alpha.10 notes do not copy the alpha.9 OCI digest" \
  docs/releases/v0.1.0-alpha.10.md \
  'sha256:b043ec5637551c2e238be15c32033ca693ecc2f765a470ba721a5986709fd692'
require_contains \
  "published alpha.10 notes pin the public source commit" \
  docs/releases/v0.1.0-alpha.10.md \
  '48f898a9e4d38e8b8c7627644b66e22076a39364'
require_contains \
  "published alpha.10 notes pin the annotated tag object" \
  docs/releases/v0.1.0-alpha.10.md \
  'b58f7561d0014228cc523b1770b5c411b017ef52'
require_contains \
  "published alpha.10 notes pin the multi-arch digest" \
  docs/releases/v0.1.0-alpha.10.md \
  'sha256:c54d1319b5173ca55540dc69e0f965a31b51cdfccb497ca77882882a16b4e477'
require_contains \
  "published alpha.10 notes pin the 22-asset contract" \
  docs/releases/v0.1.0-alpha.10.md \
  'exactly 22 host assets'
require_contains \
  "published alpha.10 notes pin sequential host builds" \
  docs/releases/v0.1.0-alpha.10.md \
  'one target at a time'
require_contains \
  "published alpha.10 notes pin sequential Docker builds" \
  docs/releases/v0.1.0-alpha.10.md \
  'strictly one after the other'
require_contains \
  "published alpha.10 notes record the zero-Actions proof" \
  docs/releases/v0.1.0-alpha.10.md \
  'GitHub Actions API returned zero runs'
require_not_contains \
  "alpha.11 notes do not copy the alpha.10 source commit" \
  docs/releases/v0.1.0-alpha.11.md \
  '48f898a9e4d38e8b8c7627644b66e22076a39364'
require_not_contains \
  "alpha.11 notes do not copy the alpha.10 OCI digest" \
  docs/releases/v0.1.0-alpha.11.md \
  'sha256:c54d1319b5173ca55540dc69e0f965a31b51cdfccb497ca77882882a16b4e477'
require_contains \
  "published alpha.11 notes pin the public source commit" \
  docs/releases/v0.1.0-alpha.11.md \
  'cd7f580a5e89ea77852468bc4fad9875f00dce61'
require_contains \
  "published alpha.11 notes pin the annotated tag object" \
  docs/releases/v0.1.0-alpha.11.md \
  'fafc41e33386ec370f3da17d24650e370d46af4e'
require_contains \
  "published alpha.11 notes pin the multi-arch digest" \
  docs/releases/v0.1.0-alpha.11.md \
  'sha256:7dbed4eff2d57e88a0fcc33d343f942454d3a1b29ea933102d050c8d7a9b1192'
require_contains \
  "published alpha.11 notes pin the 22-asset contract" \
  docs/releases/v0.1.0-alpha.11.md \
  'exactly 22 host assets'
require_contains \
  "published alpha.11 notes pin sequential host builds" \
  docs/releases/v0.1.0-alpha.11.md \
  'one target at a time'
require_contains \
  "published alpha.11 notes pin sequential Docker builds" \
  docs/releases/v0.1.0-alpha.11.md \
  'strictly one after the other'
require_contains \
  "published alpha.11 notes record the zero-Actions proof" \
  docs/releases/v0.1.0-alpha.11.md \
  'GitHub Actions API returned zero runs'
require_not_contains \
  "alpha.12 notes do not copy the alpha.11 source commit" \
  docs/releases/v0.1.0-alpha.12.md \
  'cd7f580a5e89ea77852468bc4fad9875f00dce61'
require_not_contains \
  "alpha.12 notes do not copy the alpha.11 OCI digest" \
  docs/releases/v0.1.0-alpha.12.md \
  'sha256:7dbed4eff2d57e88a0fcc33d343f942454d3a1b29ea933102d050c8d7a9b1192'
require_contains \
  "published alpha.12 notes pin the public source commit" \
  docs/releases/v0.1.0-alpha.12.md \
  'cd7bf8ab5674b402d06e36bb1c4ae9b4a5ab16a2'
require_contains \
  "published alpha.12 notes pin the annotated tag object" \
  docs/releases/v0.1.0-alpha.12.md \
  '651a018593ea2d21af2e2a50d786d7f35654be9d'
require_contains \
  "published alpha.12 notes pin the multi-arch digest" \
  docs/releases/v0.1.0-alpha.12.md \
  'sha256:5626ba43317b6341f123a5041f6d1e473db0486217c9b9912f3fd5bb41e45afa'
require_contains \
  "published alpha.12 notes pin the 22-asset contract" \
  docs/releases/v0.1.0-alpha.12.md \
  'exactly 22 host assets'
require_contains \
  "published alpha.12 notes record the zero-Actions proof" \
  docs/releases/v0.1.0-alpha.12.md \
  'GitHub Actions API returned zero runs'
require_contains \
  "Telegram operator docs pin Rich-first transport" \
  docs/channel-adapters.md \
  'Telegram is Rich-first for normal Captain replies'
require_contains \
  "agent-facing channel docs pin ephemeral progress" \
  docs/captain-tools/channel.md \
  'ephemeral operational draft after 20 seconds of real inactivity'
require_contains \
  "agent-facing changelog pins reliable ask_user cards" \
  docs/captain-tools/runtime-changelog.md \
  'Telegram `ask_user` prompts are stateful Rich cards'
require_contains \
  "alpha.6 notes pin duplicate-safe Rich fallback" \
  docs/releases/v0.1.0-alpha.6.md \
  'server failures never trigger a second send'
require_contains \
  "alpha.6 notes disclose the memory opt-out limitation" \
  docs/releases/v0.1.0-alpha.6.md \
  'core agent-loop finalizer'
require_contains \
  "alpha.7 notes disclose the retained memory opt-out limitation" \
  docs/releases/v0.1.0-alpha.7.md \
  'core agent-loop finalizer'
require_contains \
  "alpha.8 notes disclose the retained memory opt-out limitation" \
  docs/releases/v0.1.0-alpha.8.md \
  'core agent-loop finalizer'
require_contains \
  "alpha.8 notes expose Captain Forge" \
  docs/releases/v0.1.0-alpha.8.md \
  '## Captain Forge'
require_contains \
  "alpha.8 notes expose provider quota semantics" \
  docs/releases/v0.1.0-alpha.8.md \
  '## Subscription and internal quotas'
require_contains \
  "alpha.9 notes disclose the retained memory opt-out limitation" \
  docs/releases/v0.1.0-alpha.9.md \
  'core agent-loop finalizer'
require_contains \
  "alpha.9 notes expose durable workflow learning" \
  docs/releases/v0.1.0-alpha.9.md \
  '## Durable workflow learning'
require_contains \
  "alpha.9 notes expose the native release monitor" \
  docs/releases/v0.1.0-alpha.9.md \
  '## Native release monitor'
require_contains \
  "alpha.10 notes expose the security perimeter" \
  docs/releases/v0.1.0-alpha.10.md \
  '## Security perimeter'
require_contains \
  "alpha.10 notes expose guarded host execution" \
  docs/releases/v0.1.0-alpha.10.md \
  '## Guarded host execution'
require_contains \
  "alpha.10 notes expose durable operation" \
  docs/releases/v0.1.0-alpha.10.md \
  '## Durable operation and continuity'
require_contains \
  "alpha.11 notes expose the audit closure" \
  docs/releases/v0.1.0-alpha.11.md \
  '## Audit closure and execution boundary'
require_contains \
  "alpha.11 notes expose native email" \
  docs/releases/v0.1.0-alpha.11.md \
  '## Email accounts and automation'
require_contains \
  "alpha.11 notes expose external authorization honestly" \
  docs/releases/v0.1.0-alpha.11.md \
  '## External authorization boundary'
require_contains \
  "alpha.12 notes expose durable Live Runs" \
  docs/releases/v0.1.0-alpha.12.md \
  '## Durable Live Runs'
require_contains \
  "alpha.12 notes expose grounded research" \
  docs/releases/v0.1.0-alpha.12.md \
  '## Evidence-grounded Web research'
require_contains \
  "alpha.12 notes expose immutable artifacts" \
  docs/releases/v0.1.0-alpha.12.md \
  '## Immutable artifacts'
require_contains \
  "alpha.12 notes expose the managed-domain installer" \
  docs/releases/v0.1.0-alpha.12.md \
  'CAPTAIN_DOMAIN=agent.example.com'
require_contains \
  "alpha.12 notes pin the authorized destination boundary" \
  docs/releases/v0.1.0-alpha.12.md \
  'selective metadata and redacted tail are not sent'
require_not_contains \
  "current deployment docs do not call the domain rail post-Alpha 11" \
  docs/deployment/github-vps-install.md \
  'post-Alpha 11'
require_not_contains \
  "current getting-started guide does not call the domain rail post-Alpha 11" \
  docs/getting-started.md \
  'post-Alpha 11'
require_not_contains \
  "current troubleshooting does not call the domain rail post-Alpha 11" \
  docs/troubleshooting.md \
  'post-Alpha 11'
require_not_contains \
  "alpha.12 notes do not claim approval suggestion controls" \
  docs/releases/v0.1.0-alpha.12.md \
  'approval suggestion controls are available'
require_contains \
  "release readiness executes workflow audit" \
  scripts/release-readiness.sh \
  'scripts/release-workflow-audit.sh'
require_contains \
  "release readiness executes public source audit" \
  scripts/release-readiness.sh \
  'scripts/prepare-github-export.sh'
require_contains \
  "README exposes the local release publisher" \
  README.md \
  'scripts/publish-release-local.sh'
require_contains \
  "README documents deterministic Docker embeddings" \
  README.md \
  'checksum-pinned FastEmbed snapshot'
require_contains \
  "README documents sequential local release builds" \
  README.md \
  'sequentially, then assembles the GHCR index'
require_contains \
  "README links the release provenance contract" \
  README.md \
  'docs/release-provenance.md'
require_contains \
  "release provenance discloses unsigned alpha statement" \
  docs/release-provenance.md \
  'independently signed transparency-log attestation'
require_contains \
  "release provenance pins 22 uploaded assets" \
  docs/release-provenance.md \
  'asset count to 22'
for readme in README.md README.fr.md README.es.md README.zh.md; do
  require_contains \
    "$readme pins the prerelease installer" \
    "$readme" \
    'releases/download/v0.1.0-alpha.12/install.sh'
  require_contains \
    "$readme pins the immutable prerelease image" \
    "$readme" \
    'ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.12'
  require_not_contains \
    "$readme does not use GitHub latest for the prerelease" \
    "$readme" \
    'releases/latest/download/install.sh'
done
require_contains \
  "security policy supports alpha.12" \
  SECURITY.md \
  '| 0.1.0-alpha.12 | :white_check_mark: |'
require_contains \
  "security policy retires alpha.11" \
  SECURITY.md \
  '| 0.1.0-alpha.11 | :x: |'
require_contains \
  "security policy retires alpha.10" \
  SECURITY.md \
  '| 0.1.0-alpha.10 | :x: |'
require_contains \
  "local publisher derives prerelease channels" \
  scripts/publish-release-local.sh \
  'release_channel_for_version'
require_contains \
  "local publisher marks prereleases" \
  scripts/publish-release-local.sh \
  'create_args+=(--prerelease)'
require_contains \
  "runtime changelog documents deterministic Docker embeddings" \
  docs/captain-tools/runtime-changelog.md \
  'checksum-pinned FastEmbed snapshot'
require_contains \
  "memory docs pin managed runtime versions" \
  docs/captain-tools/memory.md \
  'uv 0.11.28, CPython 3.13.14'
require_contains \
  "memory docs pin all active local boot paths" \
  docs/captain-tools/memory.md \
  'Every active local kernel entrypoint'
require_contains \
  "memory docs pin fail-closed production readiness" \
  docs/captain-tools/memory.md \
  'does not claim production readiness'
require_contains \
  "memory docs pin explicit per-turn write opt-out" \
  docs/captain-tools/memory.md \
  'Explicit write opt-out contract'
require_contains \
  "CLI docs pin a single-agent fresh install" \
  docs/cli-reference.md \
  'a fresh first boot creates only the'
require_contains \
  "runtime changelog pins live model identity" \
  docs/captain-tools/runtime-changelog.md \
  'identity containing the authoritative configured provider and model for that'
require_contains \
  "provider docs pin configured-model authority" \
  docs/providers.md \
  'Every normal agent turn uses the provider and model declared on that agent.'
require_contains \
  "provider docs reject inferred fallbacks" \
  docs/providers.md \
  'never infers them from credentials present on the host.'
require_not_contains \
  "self-configure docs omit removed routing input" \
  docs/captain-tools/config-secret.md \
  '| `routing` |'
require_not_contains \
  "kernel no longer exports the routing module" \
  crates/captain-kernel/src/kernel.rs \
  'kernel_llm_routing'
require_not_contains \
  "runtime no longer exports the routing module" \
  crates/captain-runtime/src/lib.rs \
  'pub mod routing'
require_not_contains \
  "init wizard no longer offers smart model routing" \
  crates/captain-cli/src/tui/screens/init_wizard.rs \
  'Smart Model Routing'
require_contains \
  "kernel never infers fallback models" \
  crates/captain-kernel/src/kernel_model_support.rs \
  'Captain never infers alternate models from credentials present on the host.'
require_not_contains \
  "setup does not materialize the specialist template catalog" \
  crates/captain-cli/src/commands/setup.rs \
  'install_bundled_agents'
require_not_contains \
  "init does not materialize the specialist template catalog" \
  crates/captain-cli/src/commands/init.rs \
  'install_bundled_agents'
require_not_contains \
  "factory reset does not materialize the specialist template catalog" \
  crates/captain-cli/src/snapshot.rs \
  'install_bundled_agents'
require_contains \
  "runtime changelog documents manual-only release fallback" \
  docs/captain-tools/runtime-changelog.md \
  'manual fallback only'
require_contains \
  "DOC2 exposes the active CLI release artifact" \
  docs/DOCS_STATUS.md \
  'active release artifact is the cross-platform Captain CLI'
if [ "$SITE_PRESENT" = "1" ]; then
  require_contains \
    "launch site docs expose the static audit" \
    docs/deployment/launch-site.md \
    'scripts/launch-site-audit.sh'
  require_contains \
    "launch site docs expose the browser smoke" \
    docs/deployment/launch-site.md \
    'node scripts/launch-site-browser-smoke.mjs'
  require_contains \
    "launch site docs preserve the private preview gate" \
    docs/deployment/launch-site.md \
    'CAPTAIN_SITE_PUBLIC_APPROVED=1'
  require_contains \
    "launch site exposes the reviewed editorial slogan" \
    site/index.html \
    'aria-label="Unleash the future."'
  require_contains \
    "launch site labels its terminal data as representative" \
    site/index.html \
    'Interactive demo / representative data'
  require_contains \
    "terminal demo models detached run revisits" \
    site/assets/terminal-demo.js \
    'tool_run_status'
else
  pass "presentation site code is absent from the public source tree"
fi

finish
