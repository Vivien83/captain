#!/usr/bin/env bash
# Audit a Captain source tree before it becomes public.

set -euo pipefail

ROOT_DIR="${1:-$(cd "$(dirname "$0")/.." && pwd -P)}"
ROOT_DIR=$(cd "$ROOT_DIR" && pwd -P)

fail() {
  printf 'Public release audit failed: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "$1 is required"
}

pass() {
  printf '   ok %s\n' "$1"
}

need_cmd find
need_cmd gitleaks
need_cmd grep
need_cmd node
need_cmd rg

printf '== Captain public source audit\n'
printf '   root=%s\n' "$ROOT_DIR"

required_files=(
  .gitattributes
  .gitleaks.toml
  Cargo.lock
  Cargo.toml
  README.md
  SECURITY.md
  docs/releases/v0.1.0-alpha.5.md
  docs/releases/v0.1.0-alpha.4.md
  docs/releases/v0.1.0-alpha.3.md
  docs/releases/v0.1.0-alpha.2.md
  docs/releases/v0.1.0-alpha.1.md
  scripts/check-markdown-links.mjs
  scripts/install.sh
  scripts/install.ps1
)
for relative in "${required_files[@]}"; do
  [ -f "$ROOT_DIR/$relative" ] || fail "required file is missing: $relative"
done
pass "required public source files exist"

for readme in README.md README.fr.md README.es.md README.zh.md; do
  grep -Fq 'https://github.com/Vivien83/captain/releases/tag/v0.1.0-alpha.5' \
    "$ROOT_DIR/$readme" || fail "$readme does not link the immutable alpha"
  grep -Fq 'releases/download/v0.1.0-alpha.5/install.sh' \
    "$ROOT_DIR/$readme" || fail "$readme does not pin the prerelease installer"
  grep -Fq 'ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.5' \
    "$ROOT_DIR/$readme" || fail "$readme does not pin the immutable alpha image"
  if grep -Fq 'releases/latest/download/install.sh' "$ROOT_DIR/$readme"; then
    fail "$readme incorrectly uses GitHub latest for a prerelease"
  fi
  if grep -Fq '0.1.0-dev.2026-07-13a' "$ROOT_DIR/$readme"; then
    fail "$readme still exposes the private release candidate"
  fi
done
grep -Fq '### 0.1.0-alpha.5' \
  "$ROOT_DIR/docs/captain-tools/runtime-changelog.md" \
  || fail "agent-facing changelog does not identify the public alpha"
grep -Fq 'sha256:412921cd69726152235bc08614d185686ebe8a34490ee11b42a94a79e0ddc873' \
  "$ROOT_DIR/docs/releases/v0.1.0-alpha.5.md" \
  || fail "release notes do not pin the published multi-architecture image"
grep -Fq 'image: ghcr.io/vivien83/captain-agent-os:${CAPTAIN_IMAGE_TAG:-alpha}' \
  "$ROOT_DIR/docker-compose.yml" \
  || fail "Compose does not default to the public alpha channel"
CAPTAIN_RELEASE_POLICY_TEST=1 "$ROOT_DIR/scripts/publish-release-local.sh" >/dev/null \
  || fail "local release channel policy failed"
legacy_image_matches=$(rg -n --hidden \
  --glob '!.git/**' \
  --glob '!scripts/public-release-audit.sh' \
  'ghcr\.io/vivien83/captain:' \
  "$ROOT_DIR" || true)
if [ -n "$legacy_image_matches" ]; then
  printf '%s\n' "$legacy_image_matches" >&2
  fail "public tree still references the private historical image package"
fi
pass "public alpha version, installer, image, and prerelease policy are coherent"

forbidden_paths=(
  .mcp.json
  AGENTS.md
  MIGRATION.md
  start.sh
  docker-compose.personal.yml
  docker-compose.trusted.yml
  docker-compose.yolo.yml
  flake.nix
  crates/captain-migrate
  site
  deploy/captain-site.caddy
  docs/CAPTAIN_CORE_EXCELLENCE_PLAN.md
  docs/CAPTAIN_LONG_FILE_EXCEPTIONS.md
  docs/CONSCIOUSNESS-FR.md
  docs/CONSCIOUSNESS.md
  docs/PREPUBLICATION_24H_PLAN.md
  docs/autonomy-audit.md
  docs/benchmarks
  docs/deployment/launch-site.md
  docs/desktop.md
  docs/excellence-roadmap.md
  docs/installation-excellence-roadmap.md
  docs/launch-roadmap.md
  docs/mcp-a2a.md
  docs/production-checklist.md
  docs/research
  docs/SECURITY-PROFILES.md
  docs/ssh-setup.md
  scripts/build-launch-site.sh
  scripts/deploy-launch-site.sh
  scripts/hermes-vs-captain-benchmark.sh
  scripts/launch-site-audit.sh
  scripts/launch-site-browser-smoke.mjs
  skills/resawod.md
  target
  dist/releases
)
for relative in "${forbidden_paths[@]}"; do
  [ ! -e "$ROOT_DIR/$relative" ] || fail "private or generated path is present: $relative"
done
if find "$ROOT_DIR/docs" -maxdepth 1 -type f -name 'v3*.md' -print -quit | grep -q .; then
  fail "historical v3 design documents are present"
fi
pass "maintainer-only, historical, site, and generated paths are absent"

historical_nav_matches=$(rg -n \
  'MIGRATION\.md|SECURITY-PROFILES\.md|ssh-setup\.md' \
  "$ROOT_DIR/README.md" \
  "$ROOT_DIR/README.fr.md" \
  "$ROOT_DIR/README.es.md" \
  "$ROOT_DIR/README.zh.md" \
  "$ROOT_DIR/docs/README.md" \
  "$ROOT_DIR/docs/INDEX.md" || true)
if [ -n "$historical_nav_matches" ]; then
  printf '%s\n' "$historical_nav_matches" >&2
  fail "public navigation still links a historical guide"
fi
pass "public navigation contains only current guides"

for readme in README.md README.fr.md README.es.md README.zh.md; do
  grep -Fq 'docker-compose.personal.yml' "$ROOT_DIR/$readme" \
    && fail "$readme advertises a removed host-access overlay"
done
grep -Fq '/api/migrate' "$ROOT_DIR/docs/api-reference.md" \
  && fail "API reference advertises removed migration routes"
grep -Fq '/a2a/' "$ROOT_DIR/docs/api-reference.md" \
  && fail "API reference advertises frozen A2A routes"
grep -Fq '/api/clawhub' "$ROOT_DIR/docs/api-reference.md" \
  && fail "API reference advertises frozen marketplace routes"
grep -Fq 'captain migrate' "$ROOT_DIR/docs/cli-reference.md" \
  && fail "CLI reference advertises the removed migration command"
grep -Fq '[channels.slack]' "$ROOT_DIR/docs/configuration.md" \
  && fail "configuration guide advertises a frozen Slack setup"
grep -Fq 'SLACK_BOT_TOKEN' "$ROOT_DIR/docker-compose.yml" \
  && fail "Compose exposes a frozen Slack credential"
pass "removed launch, migration, host-access, and frozen-channel paths stay hidden"

unexpected_secrets=$(find "$ROOT_DIR" -type f \( \
  -name '.env' -o -name '.env.*' -o -name '*.pem' -o -name '*.key' -o \
  -name '*.p12' -o -name '*.pfx' -o -name '*.sqlite' -o -name '*.sqlite3' -o \
  -name '*.db' -o -name '*.db-wal' -o -name '*.db-shm' \
\) ! -name '.env.example' -print)
if [ -n "$unexpected_secrets" ]; then
  printf '%s\n' "$unexpected_secrets" >&2
  fail "secret or runtime-state file types are present"
fi
pass "no secret or runtime-state file types are present"

private_matches=$(rg -n --hidden \
  --glob '!.git/**' \
  --glob '!Cargo.lock' \
  --glob '!**/public-release-audit.sh' \
  --glob '!**/*.png' \
  --glob '!**/*.jpg' \
  --glob '!**/*.jpeg' \
  --glob '!**/*.svg' \
  --glob '!**/*.woff2' \
  '(/Users/vivien|Desktop/captainv2|RightNow-AI|82\.29\.175\.62|nfcsysteme?|vps-prod|dsmx83@gmail\.com)' \
  "$ROOT_DIR" || true)
if [ -n "$private_matches" ]; then
  printf '%s\n' "$private_matches" >&2
  fail "private operator path, infrastructure, or obsolete owner is present"
fi
pass "no private operator paths, infrastructure aliases, or obsolete owner remain"

automatic_actions=$(rg -n '^[[:space:]]{0,2}(push|pull_request|schedule):' \
  "$ROOT_DIR/.github/workflows" --glob '*.yml' --glob '*.yaml' || true)
if [ -n "$automatic_actions" ]; then
  printf '%s\n' "$automatic_actions" >&2
  fail "GitHub Actions must remain manual-only"
fi
pass "GitHub Actions are manual-only"

node "$ROOT_DIR/scripts/check-markdown-links.mjs" "$ROOT_DIR"
pass "local Markdown links resolve with exact path casing"

gitleaks detect \
  --source "$ROOT_DIR" \
  --no-git \
  --redact \
  --no-banner \
  --config "$ROOT_DIR/.gitleaks.toml"
pass "gitleaks found no secret in the public tree"

printf 'Captain public source audit passed.\n'
