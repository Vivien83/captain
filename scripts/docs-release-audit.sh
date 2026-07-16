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
  docs/cli-reference.md
  docs/channel-adapters.md
  docs/configuration.md
  docs/captain-tools/channel.md
  docs/captain-tools/memory.md
  docs/captain-tools/skill.md
  docs/captain-tools/runtime-changelog.md
  docs/INDEX.md
  docs/DOCS_STATUS.md
  docs/DEPLOY.md
  docs/getting-started.md
  docs/troubleshooting.md
  docs/deployment/github-vps-install.md
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
  "### 0.1.0-alpha.4"
require_contains \
  "release readiness expects the current candidate" \
  scripts/release-readiness.sh \
  '0.1.0-alpha.4'
require_contains \
  "excellence smoke expects the current candidate" \
  scripts/excellence-smoke.sh \
  '0.1.0-alpha.4'
require_contains \
  "public changelog exposes the alpha" \
  CHANGELOG.md \
  '## [0.1.0-alpha.4] - 2026-07-16'
require_contains \
  "reviewed alpha notes exist" \
  docs/releases/v0.1.0-alpha.4.md \
  '# Captain 0.1.0-alpha.4'
require_contains \
  "last published alpha.3 notes pin the public source commit" \
  docs/releases/v0.1.0-alpha.3.md \
  '13b8aca8d6d5f842cc93a23b9f03caf972f01bf1'
require_contains \
  "last published alpha.3 notes pin the multi-arch digest" \
  docs/releases/v0.1.0-alpha.3.md \
  'sha256:f7ff11969ed8b75b31c15dbc610fd785f4983f17e322f0501eea627df08ea4a2'
require_not_contains \
  "alpha.4 notes do not copy the alpha.3 source commit" \
  docs/releases/v0.1.0-alpha.4.md \
  '13b8aca8d6d5f842cc93a23b9f03caf972f01bf1'
require_not_contains \
  "alpha.4 notes do not copy the alpha.3 OCI digest" \
  docs/releases/v0.1.0-alpha.4.md \
  'sha256:f7ff11969ed8b75b31c15dbc610fd785f4983f17e322f0501eea627df08ea4a2'
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
for readme in README.md README.fr.md README.es.md README.zh.md; do
  require_contains \
    "$readme pins the public prerelease installer" \
    "$readme" \
    'releases/download/v0.1.0-alpha.4/install.sh'
  require_contains \
    "$readme pins the immutable alpha image" \
    "$readme" \
    'ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.4'
  require_not_contains \
    "$readme does not use GitHub latest for the prerelease" \
    "$readme" \
    'releases/latest/download/install.sh'
done
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
