#!/usr/bin/env bash
# DOC2 global documentation coherence audit.

set -u

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="${TMPDIR:-/tmp}/captain-docs-global-audit.$$"
PASS=0
FAIL=0

mkdir -p "$TMP_DIR" || exit 1
trap 'rm -rf "$TMP_DIR"' EXIT

README_DOCS=(
  README.md
  README.fr.md
  README.es.md
  README.zh.md
  docs/README.md
  docs/captain-tools/README.md
  crates/captain-graph/README.md
  crates/captain-graph/bindings/c/README.md
  crates/captain-graph/bindings/node/README.md
  crates/captain-graph/bindings/python/README.md
  crates/captain-graph/bindings/wasm/README.md
)

CONTRACT_DOCS=(
  "${README_DOCS[@]}"
  CHANGELOG.md
  CONTRIBUTING.md
  SECURITY.md
  docs/DOCS_STATUS.md
  docs/DEPLOY.md
  docs/INDEX.md
  docs/getting-started.md
  docs/troubleshooting.md
  docs/cli-reference.md
  docs/api-reference.md
  docs/configuration.md
  docs/channel-adapters.md
  docs/providers.md
  docs/skill-development.md
  docs/SKILL_LEARNING_V2.md
  docs/CAPTAIN_FORGE_CAPSPEC.md
  docs/architecture.md
  docs/security.md
  docs/agent-templates.md
  docs/workflows.md
  docs/captain-tools/browser.md
  docs/deployment/github-vps-install.md
  docs/deployment/vps-web-terminal.md
  docs/releases/v0.1.0-alpha.9.md
  docs/releases/v0.1.0-alpha.8.md
  docs/releases/v0.1.0-alpha.4.md
  docs/releases/v0.1.0-alpha.5.md
  docs/releases/v0.1.0-alpha.7.md
  docs/releases/v0.1.0-alpha.6.md
  docs/releases/v0.1.0-alpha.3.md
  docs/releases/v0.1.0-alpha.2.md
  docs/releases/v0.1.0-alpha.1.md
)

HISTORICAL_DOCS=(
  MIGRATION.md
  docs/SECURITY-PROFILES.md
  docs/ssh-setup.md
  docs/launch-roadmap.md
  docs/PREPUBLICATION_24H_PLAN.md
  docs/excellence-roadmap.md
  docs/installation-excellence-roadmap.md
  docs/v3.0-rename-brand.md
  docs/v3.1-captain-agent.md
  docs/v3.2-frontend-react.md
  docs/v3.3-graph-memory.md
  docs/v3.4-skill-execute.md
  docs/v3.5-workflow-crons.md
  docs/v3.6-polish-deploy.md
  docs/v3.7-prompt-pedagogy.md
  docs/v3.8-autonomous-visible.md
  docs/v3.9-computer-panel.md
  docs/v3.10-cache-efficiency.md
  docs/v3.11-projects-memory.md
  docs/v3.12-learning-engine.md
)

pass() {
  PASS=$((PASS + 1))
  printf '   ok %s\n' "$1"
}

fail() {
  FAIL=$((FAIL + 1))
  printf '   FAIL %s\n' "$1" >&2
}

finish() {
  printf '\n========================================\n'
  if [ "$FAIL" -eq 0 ]; then
    printf 'DOC2 docs global audit passed: %s checks.\n' "$PASS"
    exit 0
  fi
  printf 'DOC2 docs global audit failed: %s failed, %s passed.\n' "$FAIL" "$PASS"
  exit 1
}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    fail "missing required command: $1"
    finish
  fi
}

show_findings() {
  local file="$1"
  sed -n '1,80p' "$file"
}

require_file() {
  local file="$1"
  if [ -f "$file" ]; then
    pass "required file exists: $file"
  else
    fail "required file missing: $file"
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

scan_contract_banned() {
  local label="$1"
  local pattern="$2"
  local out="$TMP_DIR/contract-banned.txt"
  rg -n "$pattern" "${CONTRACT_DOCS[@]}" >"$out" || true
  if [ -s "$out" ]; then
    fail "$label"
    show_findings "$out"
  else
    pass "$label"
  fi
}

scan_marketplace_active_claims() {
  local raw="$TMP_DIR/marketplace-raw.txt"
  local out="$TMP_DIR/marketplace-active.txt"
  rg -n \
    'Captain Marketplace|Skill marketplace|ClawHub client|Install from ClawHub|Search ClawHub|Browse ClawHub|marketplace\.captain\.dev' \
    "${CONTRACT_DOCS[@]}" >"$raw" || true
  rg -v -i 'frozen|compat|historical|outside the active|not active|retained|migration' "$raw" >"$out" || true
  if [ -s "$out" ]; then
    fail "no active marketplace/ClawHub claims in current docs"
    show_findings "$out"
  else
    pass "no active marketplace/ClawHub claims in current docs"
  fi
}

check_historical_banners() {
  if [ "$INTERNAL_DOCS_PRESENT" != "1" ]; then
    pass "maintainer historical docs are absent from the public source tree"
    return
  fi

  local out="$TMP_DIR/historical-missing-banner.txt"
  : >"$out"
  for file in "${HISTORICAL_DOCS[@]}"; do
    if [ ! -f "$file" ]; then
      printf '%s: missing historical doc\n' "$file" >>"$out"
      continue
    fi
    if ! sed -n '1,8p' "$file" | grep -Fq 'DOC2 status: historical'; then
      printf '%s: missing DOC2 historical banner in first 8 lines\n' "$file" >>"$out"
    fi
  done
  if [ -s "$out" ]; then
    fail "historical docs have DOC2 banners"
    show_findings "$out"
  else
    pass "historical docs have DOC2 banners"
  fi
}

check_readme_inventory() {
  local declared="$TMP_DIR/readmes-declared.txt"
  local tracked="$TMP_DIR/readmes-tracked.txt"
  local out="$TMP_DIR/readmes-inventory.diff"

  printf '%s\n' "${README_DOCS[@]}" | LC_ALL=C sort >"$declared"
  git ls-files '*README*' | LC_ALL=C sort >"$tracked"
  if cmp -s "$declared" "$tracked"; then
    pass "every tracked README is covered by DOC2"
    return
  fi

  diff -u "$declared" "$tracked" >"$out" || true
  fail "every tracked README is covered by DOC2"
  show_findings "$out"
}

cd "$ROOT_DIR" || exit 1
INTERNAL_DOCS_PRESENT=0
SITE_PRESENT=0
if [ -f docs/CAPTAIN_CORE_EXCELLENCE_PLAN.md ]; then
  INTERNAL_DOCS_PRESENT=1
fi
if [ -f site/index.html ]; then
  SITE_PRESENT=1
  CONTRACT_DOCS+=(
    docs/deployment/launch-site.md
    docs/benchmarks/architecture-overview.svg
    site/index.html
    site/assets/site.css
    site/assets/site.js
    site/assets/terminal-demo.js
  )
fi
need_cmd rg
need_cmd sed
need_cmd grep
need_cmd cmp
need_cmd diff
need_cmd git
need_cmd sort
need_cmd node
need_cmd python3

printf '== DOC2 docs global audit\n'
printf '   root=%s\n' "$ROOT_DIR"

require_file docs/DOCS_STATUS.md
require_contains "DOC2 status defines current contract docs" docs/DOCS_STATUS.md "Current Contract Docs"
require_contains "DOC2 status defines agent-facing source" docs/DOCS_STATUS.md "Agent-Facing Source"
require_contains "DOC2 status defines historical docs" docs/DOCS_STATUS.md "Historical Docs"
require_contains "DOC2 status references captain_docs" docs/DOCS_STATUS.md "captain_docs"
require_contains "DOC2 defines the essential public navigation boundary" docs/DOCS_STATUS.md "public navigation exposes only current install, operation, API, security"
require_contains "DOC2 preserves runtime-bound Markdown for reproducible builds" docs/DOCS_STATUS.md 'can also be executable or build-time source'
require_contains "DOC2 pins six primary hubs" docs/DOCS_STATUS.md "exactly six primary hubs"
require_contains "DOC2 pins Control audit" docs/DOCS_STATUS.md "scripts/control-web-audit.sh"
require_contains "DOC2 pins web terminal Unicode smoke" docs/DOCS_STATUS.md "scripts/web-terminal-unicode-smoke.mjs"
require_contains "DOC2 pins release workflow audit" docs/DOCS_STATUS.md "scripts/release-workflow-audit.sh"
require_contains "DOC2 covers captain-graph binding READMEs" docs/DOCS_STATUS.md 'crates/captain-graph/bindings/{c,node,python,wasm}/README.md'
require_contains "DOC2 requires complete tracked README inventory" docs/DOCS_STATUS.md 'Every tracked `README*` file'
require_contains "DOC2 pins captain-graph binding compilation" docs/DOCS_STATUS.md "scripts/captain-graph-bindings-check.sh"
require_contains "DOC2 pins explicit Codex model decisions" docs/DOCS_STATUS.md "Availability never changes an active model by itself"
if [ "$SITE_PRESENT" = "1" ]; then
  require_contains "DOC2 pins launch site audit" docs/DOCS_STATUS.md "scripts/launch-site-audit.sh"
  require_contains "DOC2 pins launch site browser smoke" docs/DOCS_STATUS.md "scripts/launch-site-browser-smoke.mjs"
  require_contains "DOC2 covers the terminal demo module" docs/DOCS_STATUS.md "site/assets/terminal-demo.js"
else
  require_contains "DOC2 keeps presentation-site source maintainer-only" docs/DOCS_STATUS.md "source remains maintainer-only"
fi
if [ "$INTERNAL_DOCS_PRESENT" = "1" ]; then
  require_contains "desktop reference is frozen" docs/desktop.md "DOC2 status: frozen compatibility reference"
  require_contains "legacy desktop checklist is frozen" docs/production-checklist.md "DOC2 status: frozen Tauri packaging reference"
else
  pass "frozen desktop references are absent from the public source tree"
fi

check_readme_inventory
check_historical_banners

scan_contract_banned \
  "no stale fixed global counts/status in current docs" \
  '76 endpoints|76 API endpoints|All 76|40 messaging channels|40 channel adapters|40 adapters|40 channels|60 bundled skills|60 expert knowledge skills|60 skills|190\+ built-in tools|217 models|65 bundled skills|plus de 190 outils|217 modèles|65 skills packagés|más de 190 herramientas|217 modelos|65 skills incluidas|190 多个内置工具|217 个模型|65 个内置 skill|20 LLM providers|20 providers|51 builtin models|51 models|51\+ models|23 aliases|23 tools|16 security systems|967 tests|1751 tests|120\+ API routes|ALL CODE COMPLETE|Status: COMPLETE|VERIFIED'

scan_marketplace_active_claims

require_contains "CLI exposes per-agent API command" docs/cli-reference.md "captain agent api"
require_contains "API docs expose per-agent ingress" docs/api-reference.md "/hooks/agents/{id}/ingress"
require_contains "agent captain_docs expose per-agent ingress" docs/captain-tools/agent-coordination.md "/hooks/agents/{id}/ingress"
require_contains "agent guide uses the structured model table" docs/agent-templates.md '[model]'
require_contains "agent guide pins strict in/out readiness" docs/agent-templates.md '`ingress_ready` means external callers can send work'
require_not_contains "agent guide has no stale fixed catalog count" docs/agent-templates.md '30 pre-built agent templates'
require_contains "channel guide exposes only the active external tier" docs/channel-adapters.md 'active external messaging tier is deliberately small'
require_not_contains "CLI does not advertise frozen Slack setup" docs/cli-reference.md 'captain channel setup slack'
require_not_contains "CLI does not advertise the removed migration command" docs/cli-reference.md 'captain migrate'
require_contains "API reset preserves durable history" docs/api-reference.md 'only an explicit history deletion is destructive'
require_not_contains "API docs do not advertise removed migration routes" docs/api-reference.md '/api/migrate'
require_not_contains "API docs omit frozen A2A routes" docs/api-reference.md '/a2a/'
require_not_contains "API docs omit frozen marketplace routes" docs/api-reference.md '/api/clawhub'
require_not_contains "security docs do not price removed migration routes" docs/security.md '/api/migrate'
require_not_contains "public Compose omits frozen Slack credentials" docker-compose.yml 'SLACK_BOT_TOKEN'
require_not_contains "configuration guide omits frozen Slack setup" docs/configuration.md '[channels.slack]'
require_not_contains "provider guide has no copied model catalog" docs/providers.md '**Available Models:**'
require_not_contains "provider guide has no volatile price table" docs/providers.md '$/1M'
require_not_contains "skill guide omits the frozen marketplace path" docs/skill-development.md 'Frozen Marketplace'
require_contains "DOC2 classifies the Skill Learning V2 contract" docs/DOCS_STATUS.md 'docs/SKILL_LEARNING_V2.md'
require_contains "Skill Learning V2 pins the exact active model" docs/SKILL_LEARNING_V2.md 'exact active configured model'
require_contains "Skill Learning V2 confines draft authority to observed tools" docs/SKILL_LEARNING_V2.md 'canonical observed graph'
require_contains "Skill Learning V2 documents the v32 retirement boundary" docs/SKILL_LEARNING_V2.md 'Schema v32 retires the legacy sliding-window detector transactionally'
require_not_contains "Skill Learning V2 omits the retired list tool" docs/SKILL_LEARNING_V2.md 'skill_proposal_list'
require_not_contains "Skill Learning V2 omits the retired decision tool" docs/SKILL_LEARNING_V2.md 'skill_proposal_decide'
require_not_contains "config docs omit the retired skills threshold" docs/captain-tools/config-secret.md 'skills.pattern_threshold'
require_not_contains "config docs omit the retired proposer override" docs/captain-tools/config-secret.md 'skills.proposer_model'
require_contains "config docs pin authenticated workflow activation" docs/captain-tools/config-secret.md 'activation still requires an authenticated operator card'
require_contains "README points to DOC2" docs/README.md "Docs Status (DOC2)"
require_not_contains "docs navigation does not advertise frozen migration" docs/README.md 'MIGRATION.md'
for readme in README.md README.fr.md README.es.md README.zh.md; do
  require_contains "$readme pins the six operational hubs" "$readme" "Chat, Projects, Automation, Learning, Capabilities"
  require_contains "$readme documents the public alpha channel" "$readme" "ghcr.io/vivien83/captain-agent-os:alpha"
  require_contains "$readme links the immutable current release" "$readme" "https://github.com/Vivien83/captain/releases/tag/v0.1.0-alpha.9"
  require_contains "$readme pins the immutable current image" "$readme" "ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.9"
  require_contains "$readme pins the prerelease installer" "$readme" "releases/download/v0.1.0-alpha.9/install.sh"
  require_contains "$readme opens the Control root" "$readme" 'http://127.0.0.1:50051/'
  require_not_contains "$readme does not use GitHub latest for a prerelease" "$readme" "releases/latest/download/install.sh"
  require_not_contains "$readme does not require a registry token" "$readme" "GHCR_TOKEN"
  require_not_contains "$readme has no private candidate version" "$readme" "0.1.0-dev.2026-07-13a"
  require_contains "$readme documents local release publication" "$readme" "scripts/publish-release-local.sh"
  require_contains "$readme documents deterministic Docker embeddings" "$readme" "FastEmbed"
  require_not_contains "$readme does not link historical security profiles" "$readme" "SECURITY-PROFILES.md"
  require_not_contains "$readme does not advertise removed host-access overlays" "$readme" "docker-compose.personal.yml"
  require_not_contains "$readme does not advertise frozen A2A" "$readme" "mcp-a2a.md"
done
require_contains "English README discloses missing notarization" README.md "not Apple-notarized"
require_contains "French README discloses missing notarization" README.fr.md "ne sont pas notarisés"
require_contains "Spanish README discloses missing notarization" README.es.md "no están notarizados"
require_contains "Chinese README discloses missing notarization" README.zh.md "尚未经过 Apple notarization"
require_contains "English README documents proactive Codex discovery" README.md "an hourly refresh surfaces newly listed models"
require_contains "French README documents proactive Codex discovery" README.fr.md "une actualisation horaire signale les nouveaux modèles"
require_contains "Spanish README documents proactive Codex discovery" README.es.md "una actualización cada hora muestra los modelos nuevos"
require_contains "Chinese README documents proactive Codex discovery" README.zh.md "每小时刷新一次目录"
require_contains "English README separates provider subscription quotas" README.md "provider-owned subscription windows"
require_contains "French README separates provider subscription quotas" README.fr.md "fenêtres d'abonnement gérées par le fournisseur"
require_contains "Spanish README separates provider subscription quotas" README.es.md "ventanas de suscripción gestionadas por el proveedor"
require_contains "Chinese README separates provider subscription quotas" README.zh.md "供应商管理的订阅窗口"
require_contains "English README scopes compact provider gauges" README.md "limit matching the active model"
require_contains "French README scopes compact provider gauges" README.fr.md "hors modèle actif"
require_contains "Spanish README scopes compact provider gauges" README.es.md "fuera del modelo activo"
require_contains "Chinese README scopes compact provider gauges" README.zh.md "不属于当前模型"
require_contains "English README exposes readable native capabilities" README.md "Readable native capabilities"
require_contains "French README exposes readable native capabilities" README.fr.md "Capacités natives lisibles"
require_contains "Spanish README exposes readable native capabilities" README.es.md "Capacidades nativas legibles"
require_contains "Chinese README exposes readable native capabilities" README.zh.md "人类可读的原生能力"
require_contains "English README documents the native 12-hour release monitor" README.md "after startup and then every 12"
require_contains "French README documents the native 12-hour release monitor" README.fr.md "les 12 heures"
require_contains "Spanish README documents the native 12-hour release monitor" README.es.md "cada 12 horas"
require_contains "Chinese README documents the native 12-hour release monitor" README.zh.md "之后每 12 小时检查一次"
require_contains "deployment pins model-independent versioned update decisions" docs/DEPLOY.md "Callback decisions bypass the model"
require_contains "CLI documents the durable release monitor projection" docs/cli-reference.md "jq '.runtime_update'"
require_contains "API status documents runtime update state" docs/api-reference.md '`runtime_update` | Last successful release check'
require_contains "Telegram docs pin explicit update operator identity" docs/channel-adapters.md 'explicitly listed numeric Telegram user; `allowed_users = ["*"]`'
require_contains "meta docs distinguish the native release monitor" docs/captain-tools/meta.md "distinct from the native release monitor"
require_contains "runtime changelog pins twelve-hour release checks" docs/captain-tools/runtime-changelog.md "every 12 hours"
require_contains "public changelog records the native release monitor" CHANGELOG.md "compatible official release channel after startup"
require_contains "DOC2 classifies the unreleased release monitor" docs/DOCS_STATUS.md "The native Captain release monitor checks after startup and every 12 hours"
require_contains "kernel boots the native release monitor" crates/captain-kernel/src/kernel_background_startup.rs "spawn_runtime_update_monitor"
require_contains "kernel uses an exact twelve-hour update interval" crates/captain-kernel/src/release_updates.rs '12 * 60 * 60 * 1_000'
require_contains "Telegram update callbacks precede workflow and session routing" crates/captain-channels/src/bridge.rs "try_resolve_runtime_update_operator_callback().await"
require_contains "runtime updates preserve the exact release tag" crates/captain-kernel/src/release_updates_state.rs "release_tag: release.tag_name.clone()"
require_contains "runtime updates distinguish host container and manual modes" crates/captain-types/src/release_update.rs "pub enum RuntimeUpdateInstallMode"
require_contains "current runtime changelog entry is pinned" docs/captain-tools/runtime-changelog.md "### 0.1.0-alpha.9"
require_contains "public changelog entry is pinned" CHANGELOG.md "## [0.1.0-alpha.9] - 2026-07-22"
require_contains "reviewed current alpha release notes exist" docs/releases/v0.1.0-alpha.9.md "# Captain 0.1.0-alpha.9"
require_contains "historical alpha.8 release notes remain available" docs/releases/v0.1.0-alpha.8.md "# Captain 0.1.0-alpha.8"
require_contains "historical alpha.7 release notes remain available" docs/releases/v0.1.0-alpha.7.md "# Captain 0.1.0-alpha.7"
require_contains "historical alpha.6 release notes remain available" docs/releases/v0.1.0-alpha.6.md "# Captain 0.1.0-alpha.6"
require_contains "historical alpha.5 release notes remain available" docs/releases/v0.1.0-alpha.5.md "# Captain 0.1.0-alpha.5"
require_contains "Telegram docs pin Rich-first transport" docs/channel-adapters.md "Telegram is Rich-first for normal Captain replies"
require_contains "channel family docs pin stateful ask_user" docs/captain-tools/channel.md '`ask_user` questions are stateful Rich cards'
require_contains "historical alpha.4 release notes remain available" docs/releases/v0.1.0-alpha.4.md "# Captain 0.1.0-alpha.4"
require_contains "historical alpha.3 release notes remain available" docs/releases/v0.1.0-alpha.3.md "# Captain 0.1.0-alpha.3"
require_contains "DOC2 records the published alpha.8 provenance" docs/DOCS_STATUS.md "d82f120153b8e83e9be82df6748f928f8d4aa6b9"
require_contains "DOC2 records the published alpha.8 multi-arch digest" docs/DOCS_STATUS.md "sha256:af32a605de0a019482ff3aadcee07179171630ccfb45c9b88fbcf135d2680230"
require_contains "agent changelog records the published alpha.8 multi-arch digest" docs/captain-tools/runtime-changelog.md "sha256:af32a605de0a019482ff3aadcee07179171630ccfb45c9b88fbcf135d2680230"
require_contains "DOC2 records the published alpha.9 provenance" docs/DOCS_STATUS.md "1248c5928dd4968b6ff7c62ef79a607fb8d94348"
require_contains "DOC2 records the published alpha.9 multi-arch digest" docs/DOCS_STATUS.md "sha256:b043ec5637551c2e238be15c32033ca693ecc2f765a470ba721a5986709fd692"
require_contains "agent changelog records the published alpha.9 multi-arch digest" docs/captain-tools/runtime-changelog.md "sha256:b043ec5637551c2e238be15c32033ca693ecc2f765a470ba721a5986709fd692"
require_contains "DOC2 identifies the alpha.9 public release" docs/DOCS_STATUS.md '`v0.1.0-alpha.9` is the current public prerelease'
require_contains "DOC2 retains the alpha.8 public history" docs/DOCS_STATUS.md '`v0.1.0-alpha.8` is the previous public prerelease'
require_contains "DOC2 retains the alpha.7 source provenance" docs/DOCS_STATUS.md "dc2f64603eff708a8eab5735121cfc1a2d39386f"
require_contains "DOC2 retains the alpha.7 multi-arch digest" docs/DOCS_STATUS.md "sha256:e49e1ad02d6a65742343aaf7abcd1c4fcfd277dab605d3d284830f03c7d42354"
require_contains "agent changelog retains the alpha.7 multi-arch digest" docs/captain-tools/runtime-changelog.md "sha256:e49e1ad02d6a65742343aaf7abcd1c4fcfd277dab605d3d284830f03c7d42354"
require_contains "DOC2 retains the alpha.7 public history" docs/DOCS_STATUS.md '`v0.1.0-alpha.7` is an earlier public prerelease'
require_contains "DOC2 discloses the alpha.9 memory opt-out limitation" docs/DOCS_STATUS.md "core agent-loop finalizer to write one local episodic interaction"
require_contains "DOC2 discloses the alpha.8 memory opt-out limitation" docs/DOCS_STATUS.md "the core agent-loop finalizer still writes its local episodic interaction"
require_contains "memory docs disclose the alpha.8 finalizer limitation" docs/captain-tools/memory.md "agent-loop finalizer still stores one local episodic interaction"
require_contains "historical alpha.2 release notes remain available" docs/releases/v0.1.0-alpha.2.md "# Captain 0.1.0-alpha.2"
require_contains "historical alpha release notes remain available" docs/releases/v0.1.0-alpha.1.md "# Captain 0.1.0-alpha.1"
require_contains "runtime changelog marks 07-12b as published" docs/captain-tools/runtime-changelog.md "is the published release that includes the aligned"
require_not_contains "runtime changelog has no stale 07-12b candidate claim" docs/captain-tools/runtime-changelog.md 'aligned candidate is `0.1.0-dev.2026-07-12b`'
require_contains "runtime changelog supersedes old WEB1 claim" docs/captain-tools/runtime-changelog.md "superseded by this entry"
require_contains "CLI documents all Automation tabs" docs/cli-reference.md "Workflows, Triggers, Crons, Approbations, and Webhooks"
require_contains "CLI quick init is Codex-first" docs/cli-reference.md "Reuses Codex subscription credentials first"
require_contains "API status documents runtime health" docs/api-reference.md '`runtime_health`'
require_contains "API status documents interrupted runs" docs/api-reference.md "running/completed/failed/cancelled/interrupted"
require_contains "API detail documents recoverable failure count" docs/api-reference.md '"failure_count"'
require_contains "API status separates failures from panics" docs/api-reference.md '`panic_count` is reserved for actual caught task panics'
require_contains "runtime changelog separates failures from panics" docs/captain-tools/runtime-changelog.md '`failure_count`; they no longer poison'
require_contains "runtime changelog pins xterm Unicode widths" docs/captain-tools/runtime-changelog.md "Unicode 11 width provider"
require_contains "runtime changelog pins Codex model consent" docs/captain-tools/runtime-changelog.md "Captain never enables a newly visible"
require_contains "provider guide pins hourly Codex refresh" docs/providers.md "then once per hour"
require_contains "provider guide pins safe Codex session choices" docs/providers.md "Nouvelle session"
require_contains "provider guide pins Codex catalog protocol" docs/providers.md '`client_version=1.0.0`'
require_contains "provider guide pins dynamic model context" docs/providers.md 'Every turn resolves the configured provider/model against the live runtime'
require_contains "provider guide distinguishes Codex active and maximum windows" docs/providers.md '`max_context_window` is an upper bound'
require_contains "provider guide uses the official Codex account quota endpoint" docs/providers.md '`/backend-api/wham/usage`'
require_contains "provider guide pins live Codex quota signals" docs/providers.md '`codex.rate_limits` stream events'
require_contains "provider guide rejects inferred unlimited quota" docs/providers.md 'means that no current official observation exists; it never means unlimited'
require_contains "CLI status separates provider subscription quota" docs/cli-reference.md 'provider-reported subscription windows'
require_contains "CLI scopes compact gauges to the active model" docs/cli-reference.md 'limit family matching that model'
require_contains "API status exposes provider subscription observations" docs/api-reference.md '`provider_subscriptions` has stable states'
require_contains "API pins local-only quota surface polling" docs/api-reference.md 'cadence does not call the provider'
require_contains "API documents typed quota failures" docs/api-reference.md '"scope": "agent_hourly_tokens"'
require_contains "API documents provider subscription scope" docs/api-reference.md '`scope` is `provider_subscription`'
require_contains "runtime changelog pins durable rolling token quota" docs/captain-tools/runtime-changelog.md 'internal rolling one-hour'
require_contains "runtime changelog pins official Codex quota SSE" docs/captain-tools/runtime-changelog.md '`codex.rate_limits` stream events'
require_contains "runtime changelog pins active-model quota gauges" docs/captain-tools/runtime-changelog.md 'compact band names the active model'
require_contains "DOC2 separates internal and provider quotas" docs/DOCS_STATUS.md "provider-owned subscription allowances"
require_contains "DOC2 certifies shared quota status surfaces" docs/DOCS_STATUS.md 'surfaces refresh from Captain locally'
require_contains "public changelog records live provider quota source" CHANGELOG.md "official response-header/SSE signals"
require_contains "public changelog records active-model quota gauges" CHANGELOG.md 'live gauges only to provider-wide windows'
require_contains "architecture keeps quota provider calls daemon-owned" docs/architecture.md 'No operator surface calls Codex itself'
require_contains "Control web renders quota progress bars" crates/captain-api/static/js/app/views/Chat.js 'role="progressbar"'
require_contains "quota visual smoke covers desktop and mobile" scripts/provider-quota-surfaces-smoke.mjs "name: 'desktop'"
require_contains "API distinguishes context capacity from occupancy" docs/api-reference.md '`estimated_context_tokens` approximates the stored transcript'
require_contains "architecture resolves context on every turn" docs/architecture.md "Before every turn"
require_contains "runtime uses the active Codex context field first" crates/captain-runtime/src/model_catalog_codex.rs '.context_window'
require_contains "architecture pins power-loss-safe SQLite commits" docs/architecture.md '`synchronous=FULL`'
require_contains "architecture pins macOS full-fsync state files" docs/architecture.md '`F_FULLFSYNC` after `fsync` on macOS'
require_contains "deployment documents the isolated SIGKILL proof" docs/DEPLOY.md 'scripts/persistence-power-loss-smoke.sh'
require_contains "runtime changelog exposes the durable commit boundary" docs/captain-tools/runtime-changelog.md 'explicit power-loss commit boundary'
require_contains "public changelog exposes the durable commit boundary" CHANGELOG.md 'explicit power-loss commit boundary'
require_contains "SQLite runtime enables full synchronous commits" crates/captain-memory/src/substrate.rs 'PRAGMA synchronous=FULL'
require_contains "Captain state files use a central durable primitive" crates/captain-types/src/durable_fs.rs 'pending.persist(path)'
require_contains "provider guide pins configured-model authority" docs/providers.md 'Every normal agent turn uses the provider and model declared on that agent.'
require_contains "provider guide routes specialization through explicit sub-agents" docs/providers.md 'explicit specialist sub-agent'
require_contains "provider guide rejects inferred fallback models" docs/providers.md 'never infers them from credentials present on the host.'
require_not_contains "provider guide does not advertise model routing" docs/providers.md 'Fallbacks and Routing'
require_not_contains "self-configure docs omit removed routing input" docs/captain-tools/config-secret.md '| `routing` |'
require_contains "DOC2 pins configured-model authority" docs/DOCS_STATUS.md "Each agent's configured provider/model is authoritative"
require_not_contains "kernel no longer exports complexity routing" crates/captain-kernel/src/kernel.rs 'kernel_llm_routing'
require_not_contains "runtime no longer exports complexity routing" crates/captain-runtime/src/lib.rs 'pub mod routing'
require_not_contains "init wizard no longer offers complexity routing" crates/captain-cli/src/tui/screens/init_wizard.rs 'Smart Model Routing'
require_contains "runtime pins Codex catalog protocol" crates/captain-runtime/src/model_catalog_codex.rs 'CODEX_CATALOG_CLIENT_VERSION: &str = "1.0.0"'
require_contains "browser docs pin same-model visual analysis" docs/captain-tools/browser.md "same active model"
require_contains "browser docs reject a secondary Vision agent" docs/captain-tools/browser.md "does not call a separate Vision agent"
require_contains "browser docs pin capture-only semantics" docs/captain-tools/browser.md "pixels are not injected into the model context"
require_contains "DOC2 pins native same-model images" docs/DOCS_STATUS.md "Images and prompted browser screenshots stay on the active conversation model"
require_contains "runtime validates active-model image support" crates/captain-kernel/src/capability_routing.rs "ensure_active_model_supports"
require_contains "runtime discloses no hidden image delegation" crates/captain-kernel/src/capability_routing.rs "did not send the image to another agent or provider"
require_not_contains "runtime has no automatic Vision-agent manifest" crates/captain-kernel/src/capability_routing.rs "build_vision_agent_manifest"
require_not_contains "runtime has no hidden image spawn path" crates/captain-kernel/src/capability_routing.rs "SpawnAndDelegate"
require_contains "memory docs pin every active local boot preflight" docs/captain-tools/memory.md "Every active local kernel entrypoint"
require_contains "daemon boot checks managed memory" crates/captain-cli/src/commands/daemon.rs "ensure_native_mempalace_for_config"
require_contains "direct CLI boot checks managed memory" crates/captain-cli/src/cli_runtime.rs "prepare_kernel_config"
require_contains "TUI boot checks managed memory" crates/captain-cli/src/tui/event.rs "prepare_kernel_config"
require_contains "Captain MCP boot checks managed memory" crates/captain-cli/src/mcp.rs "prepare_kernel_config"
require_contains "host installer provisions managed memory" scripts/install.sh '"$INSTALL_DIR/captain" memory install'
require_contains "Windows installer provisions managed memory" scripts/install.ps1 '& $installedExe memory install'
require_contains "container boot repairs managed memory" docker-entrypoint.sh "captain memory install --force"
require_contains "Control declares the Captain favicon" crates/captain-api/src/webchat.rs 'every_web_surface_declares_the_captain_favicon'
require_contains "favicon endpoint serves embedded Captain PNG" crates/captain-api/src/webchat.rs 'favicon_endpoint_serves_the_embedded_captain_png'
require_contains "API documents Codex update inspection" docs/api-reference.md "GET /api/models/updates"
require_contains "API documents Codex update decisions" docs/api-reference.md "POST /api/models/updates/decision"
require_contains "API routes mount Codex update inspection" crates/captain-api/src/server_capability_routes.rs '"/api/models/updates"'
require_contains "API routes mount Codex update decisions" crates/captain-api/src/server_capability_routes.rs '"/api/models/updates/decision"'
require_contains "Control exposes explicit Codex keep" crates/captain-api/static/js/app/components/Shell.js '>Conserver</button>'
require_contains "Control exposes explicit Codex switch" crates/captain-api/static/js/app/components/Shell.js '>Basculer</button>'
require_contains "web terminal deployment pins Unicode addon" docs/deployment/vps-web-terminal.md "addon-unicode11 0.9.0"
require_contains "API documents scoped persisted turns" docs/api-reference.md "without changing the agent's globally active session"
require_contains "API documents detached session creation" docs/api-reference.md '`activate` defaults to `true`'
require_contains "runtime changelog pins session isolation" docs/captain-tools/runtime-changelog.md "owner and continues that transcript without switching"
require_contains "web terminal deployment rejects UUID inference" docs/deployment/vps-web-terminal.md "never assumes that a UUID-shaped terminal ID"
require_contains "architecture pins reopenable reset" docs/architecture.md "Session reset creates a new default but preserves the prior"
require_contains "API slash new preserves prior history" docs/api-reference.md "the previous session remains available in history"
require_contains "channel slash new reports preserved history" crates/captain-api/src/channel_bridge.rs "The previous session remains available in history"
require_not_contains "session reset messages never claim durable history was cleared" crates/captain-api/src/ws.rs "Session reset. Chat history cleared."
require_contains "DOC2 pins durable independently addressable chats" docs/DOCS_STATUS.md "Persisted chat sessions are durable and independently addressable"
require_contains "API pins source-independent session catalog" docs/api-reference.md "source-independent catalog used by Web Control, TUI, CLI and Desktop"
require_contains "architecture pins one cross-surface catalog" docs/architecture.md "One cross-surface catalog"
require_contains "runtime changelog pins global multi-agent session drawer" docs/captain-tools/runtime-changelog.md 'drawer now queries global `/api/sessions`'
require_contains "web deployment pins every session provenance" docs/deployment/vps-web-terminal.md "conversations created by Web, TUI, CLI, Desktop or API"
require_contains "web deployment pins fresh PTY restore" docs/deployment/vps-web-terminal.md "Selecting a history row creates a fresh PTY"
if [ "$INTERNAL_DOCS_PRESENT" = "1" ]; then
  require_contains "desktop reuses canonical session history" docs/desktop.md "does not own a separate chat history"
else
  require_contains "public docs pin cross-surface session history" docs/architecture.md "One cross-surface catalog"
fi
require_contains "DOC2 pins legacy TUI session import" docs/DOCS_STATUS.md '`$CAPTAIN_HOME/sessions/*/*.json` files'
require_contains "CLI documents cross-surface session resume" docs/cli-reference.md '/resume <UUID|unique-prefix|title>'
require_contains "architecture pins one-shot legacy import markers" docs/architecture.md 'sidecar `.json.imported` marker'
require_contains "architecture pins bounded HTTP shutdown" docs/architecture.md "long-lived HTTP connections"
require_contains "architecture pins bounded channel shutdown" docs/architecture.md "gives adapters a separate 15-second"
require_contains "runtime changelog pins bounded HTTP shutdown" docs/captain-tools/runtime-changelog.md "WebSocket/SSE connections 15 seconds"
require_contains "runtime changelog pins bounded channel shutdown" docs/captain-tools/runtime-changelog.md "separate 15-second drain period"
require_contains "JavaScript SDK loads canonical session transcripts" sdk/javascript/index.js 'async get(id)'
require_contains "JavaScript SDK types expose session transcript loading" sdk/javascript/index.d.ts 'get(id: string): Promise<unknown>'
require_contains "Python SDK loads canonical session transcripts" sdk/python/captain_client.py 'def get(self, session_id: str)'
if node --check sdk/javascript/index.js >/dev/null 2>&1; then
  pass "JavaScript SDK parses"
else
  fail "JavaScript SDK parses"
fi
if PYTHONPYCACHEPREFIX="$TMP_DIR/pycache" python3 -m py_compile sdk/python/captain_client.py; then
  pass "Python SDK parses"
else
  fail "Python SDK parses"
fi
require_contains "metrics docs expose recoverable failure counter" docs/captain-tools/runtime-changelog.md '`captain_agent_failures_total`'
require_contains "API tools expose schemas" docs/api-reference.md '"input_schema"'
require_contains "API workflow history is scoped" docs/api-reference.md "strictly scoped to the requested workflow"
require_contains "workflow guide exposes Control hub" docs/workflows.md "Automation > Workflows"
require_contains "workflow guide pins newest-first history" docs/workflows.md "orders results newest-first"
require_contains "shell docs pin fail-closed parallelism" docs/captain-tools/shell-process.md "The classifier fails closed"
require_contains "shell docs pin interrupted persistence" docs/captain-tools/shell-process.md 'becomes `interrupted` after a restart'
require_not_contains "architecture has no stale schema v5 claim" docs/architecture.md "schema v5"
require_not_contains "architecture has no removed migrate crate" docs/architecture.md "captain-migrate"
require_not_contains "architecture has no stale endpoint count" docs/architecture.md "76 endpts"
require_not_contains "workflow guide has no unscoped-history claim" docs/workflows.md "not filtered by workflow ID"
require_not_contains "CLI has no Groq fallback claim" docs/cli-reference.md "Falls back to Groq"
require_contains "tool index points to split live definitions" docs/captain-tools/README.md 'crates/captain-runtime/src/tools/'
require_not_contains "tool index has no stale monolith source claim" docs/captain-tools/README.md 'description change in `crates/captain-runtime/src/tool_runner.rs`'
require_contains "DOC2 classifies the CapSpec contract" docs/DOCS_STATUS.md 'docs/CAPTAIN_FORGE_CAPSPEC.md'
require_contains "DOC2 pins the certified CapSpec implementation commit" docs/DOCS_STATUS.md '38ecebaf4e34fcf955c99ee13682b54a70e1c938'
require_contains "docs index exposes the CapSpec contract" docs/INDEX.md 'Captain Forge / CapSpec'
require_file docs/evidence/CAPSPEC1_REAL_CERTIFICATION_2026-07-18.md
require_contains "CapSpec certificate records all process checks" docs/evidence/CAPSPEC1_REAL_CERTIFICATION_2026-07-18.md 'Checks: **130 passed**'
require_contains "CapSpec certificate records all durable runs" docs/evidence/CAPSPEC1_REAL_CERTIFICATION_2026-07-18.md 'Durable runs: **14**'
require_contains "CapSpec certificate pins its implementation commit" docs/evidence/CAPSPEC1_REAL_CERTIFICATION_2026-07-18.md '38ecebaf4e34fcf955c99ee13682b54a70e1c938'
require_contains "CapSpec certificate is reproducible" docs/evidence/CAPSPEC1_REAL_CERTIFICATION_2026-07-18.md 'scripts/capspec-real-certification.sh'
require_contains "CapSpec contract pins native ToolRunner dispatch" docs/CAPTAIN_FORGE_CAPSPEC.md 'Each primitive step re-enters the'
require_contains "CapSpec contract pins the agent approval boundary" docs/CAPTAIN_FORGE_CAPSPEC.md 'No agent-facing action can approve'
require_contains "CapSpec contract exposes the authenticated operator API" docs/CAPTAIN_FORGE_CAPSPEC.md 'POST /api/capabilities/native/{name}/decision'
require_contains "CapSpec contract exposes native Telegram decisions" docs/CAPTAIN_FORGE_CAPSPEC.md 'Telegram is also a native operator surface'
require_contains "CapSpec Telegram decisions bypass session dispatch" docs/CAPTAIN_FORGE_CAPSPEC.md 'before any session dispatch'
require_contains "CapSpec contract pins exact uncertain API" docs/CAPTAIN_FORGE_CAPSPEC.md 'POST /api/capabilities/native/runs/{run_id}/decision'
require_contains "CapSpec resume cannot expand pinned authority" docs/CAPTAIN_FORGE_CAPSPEC.md 'can never expand its pinned'
require_contains "CapSpec resume intent is atomic and crash recoverable" docs/CAPTAIN_FORGE_CAPSPEC.md 'persist an operator-resume intent in that same'
require_contains "CapSpec API distinguishes explicit null from omission" docs/api-reference.md 'explicit JSON `null` is valid, but an absent field is not'
require_contains "CapSpec contract pins native-first TUI" docs/CAPTAIN_FORGE_CAPSPEC.md 'TUI Capabilities hub likewise'
require_contains "CapSpec TUI decisions are direct" docs/CAPTAIN_FORGE_CAPSPEC.md 'It never delegates an'
require_contains "CapSpec TUI decisions bypass the model" docs/CAPTAIN_FORGE_CAPSPEC.md 'operator decision to the model'
require_contains "CapSpec contract pins native-first Control" docs/CAPTAIN_FORGE_CAPSPEC.md 'promotes `Natives` as its first tab'
require_contains "API reference exposes native CapSpec management" docs/api-reference.md '## Native Capability Endpoints'
require_contains "API reference pins exact-hash CapSpec decisions" docs/api-reference.md 'A stale or mismatched hash is'
require_contains "API reference pins exact uncertain-run decisions" docs/api-reference.md 'The run/node status, attempt, and tool-use ID are compared'
require_contains "meta docs expose controlled CapSpec authoring" docs/captain-tools/meta.md 'capability_forge'
require_contains "meta docs expose capfile discovery" docs/captain-tools/meta.md '`capfile_tool` and status `active_native`'
require_contains "runtime changelog exposes native CapSpec dispatch" docs/captain-tools/runtime-changelog.md 'Captain Forge native capability runtime'
require_contains "runtime changelog records CapSpec process certification" docs/captain-tools/runtime-changelog.md 'passed 130 checks across 14 durable'
require_contains "architecture includes the CapSpec crate" docs/architecture.md '**captain-capspec**'
require_contains "security docs pin CapSpec authority intersection" docs/security.md 'The readable `.captain` file cannot grant a'
require_not_contains "CapSpec contract has no stale open-matrix claim" docs/CAPTAIN_FORGE_CAPSPEC.md 'broad real certification matrix is still open'
require_not_contains "runtime changelog has no stale open CapSpec gate" docs/captain-tools/runtime-changelog.md 'broad real certification matrix remains required'
if [ "$SITE_PRESENT" = "1" ]; then
  require_contains "launch site restores the editorial slogan" site/index.html 'aria-label="Unleash the future."'
  require_contains "launch site labels representative terminal data" site/index.html "Interactive demo / representative data"
  require_contains "terminal demo revisits detached work" site/assets/terminal-demo.js "tool_run_status"
else
  pass "presentation site code is absent from the public source tree"
fi

require_file crates/captain-graph/README.md
require_file crates/captain-graph/bindings/c/README.md
require_file crates/captain-graph/bindings/node/README.md
require_file crates/captain-graph/bindings/python/README.md
require_file crates/captain-graph/bindings/wasm/README.md
require_contains "graph README routes each language to its binding contract" crates/captain-graph/README.md "binding-specific README"
require_not_contains "graph README has no stale Python class" crates/captain-graph/README.md "HoraGraph"
require_not_contains "graph README has no stale WASM class" crates/captain-graph/README.md "HoraWasm"
require_not_contains "graph README has no stale C constructor" crates/captain-graph/README.md "hora_new_memory"
require_not_contains "graph README has no fixed test count" crates/captain-graph/README.md "310 tests"
require_not_contains "graph README has no false zero-unsafe claim" crates/captain-graph/README.md "zero unsafe"

require_contains "C binding README uses exported constructor" crates/captain-graph/bindings/c/README.md "HoraCore *graph = hora_new(0);"
require_contains "C header exports documented constructor" crates/captain-graph/bindings/c/hora_graph_core.h "HoraCore *hora_new(uint16_t embedding_dims);"
require_not_contains "C binding README has no removed hora_core API" crates/captain-graph/bindings/c/README.md "hora_core_"

require_contains "Node binding README uses factory constructor" crates/captain-graph/bindings/node/README.md "HoraCore.newMemory()"
require_contains "Node types export documented factory" crates/captain-graph/bindings/node/index.d.ts "static newMemory("
require_contains "Node binding README uses fact API" crates/captain-graph/bindings/node/README.md "graph.addFact("
require_not_contains "Node binding README has no public constructor claim" crates/captain-graph/bindings/node/README.md "new HoraCore("
require_not_contains "Node binding README has no removed edge API" crates/captain-graph/bindings/node/README.md "addEdge("

require_contains "Python binding README uses factory constructor" crates/captain-graph/bindings/python/README.md "HoraCore.new_memory()"
require_contains "Python type hints export documented factory" crates/captain-graph/bindings/python/hora_graph_core/hora_graph_core.pyi "def new_memory("
require_contains "Python binding README uses fact API" crates/captain-graph/bindings/python/README.md "graph.add_fact("
require_contains "Python binding README pins supported CPython range" crates/captain-graph/bindings/python/README.md "CPython 3.9 through 3.13"
require_contains "Python package metadata matches supported range" crates/captain-graph/bindings/python/pyproject.toml 'requires-python = ">=3.9,<3.14"'
require_not_contains "Python binding README has no public constructor claim" crates/captain-graph/bindings/python/README.md "HoraCore()"
require_not_contains "Python binding README has no removed edge API" crates/captain-graph/bindings/python/README.md "add_edge("

require_contains "WASM binding README uses factory constructor" crates/captain-graph/bindings/wasm/README.md "HoraCore.newMemory()"
require_contains "WASM source exports documented factory" crates/captain-graph/bindings/wasm/src/lib.rs 'js_name = "newMemory"'
require_contains "WASM binding README uses fact API" crates/captain-graph/bindings/wasm/README.md "graph.addFact("
require_not_contains "WASM binding README has no public constructor claim" crates/captain-graph/bindings/wasm/README.md "new HoraCore("
require_not_contains "WASM binding README has no Rust-style entity API" crates/captain-graph/bindings/wasm/README.md "add_entity("
require_not_contains "WASM binding README has no removed edge API" crates/captain-graph/bindings/wasm/README.md "add_edge("

require_contains "C binding is an isolated Cargo workspace" crates/captain-graph/bindings/c/Cargo.toml "[workspace]"
require_contains "Node binding is an isolated Cargo workspace" crates/captain-graph/bindings/node/Cargo.toml "[workspace]"
require_contains "Python binding is an isolated Cargo workspace" crates/captain-graph/bindings/python/Cargo.toml "[workspace]"
require_contains "WASM binding is an isolated Cargo workspace" crates/captain-graph/bindings/wasm/Cargo.toml "[workspace]"
require_file scripts/captain-graph-bindings-check.sh
require_contains "release readiness compiles captain-graph bindings" scripts/release-readiness.sh 'scripts/captain-graph-bindings-check.sh'
require_file scripts/control-web-audit.sh
require_file scripts/release-workflow-audit.sh
require_file scripts/publish-release-local.sh
require_file scripts/prepare-docker-embedding-cache.sh
require_file scripts/prepare-github-export.sh
require_file scripts/public-release-audit.sh
require_file scripts/check-markdown-links.mjs
require_file scripts/public-export-smoke.sh
require_contains "release readiness audits the public source export" scripts/release-readiness.sh 'scripts/prepare-github-export.sh'
require_contains "DOC2 pins the public source audit" docs/DOCS_STATUS.md 'scripts/public-release-audit.sh'
require_contains "DOC2 pins the public export smoke" docs/DOCS_STATUS.md 'scripts/public-export-smoke.sh'

finish
