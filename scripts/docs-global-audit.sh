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
  MIGRATION.md
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
  docs/architecture.md
  docs/security.md
  docs/SECURITY-PROFILES.md
  docs/agent-templates.md
  docs/mcp-a2a.md
  docs/workflows.md
  docs/captain-tools/browser.md
  docs/deployment/github-vps-install.md
  docs/deployment/vps-web-terminal.md
  docs/releases/v0.1.0-alpha.2.md
  docs/releases/v0.1.0-alpha.1.md
)

HISTORICAL_DOCS=(
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
  docs/v3.13-skill-synthesizer.md
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
require_contains "README points to DOC2" docs/README.md "Docs Status (DOC2)"
for readme in README.md README.fr.md README.es.md README.zh.md; do
  require_contains "$readme pins the six operational hubs" "$readme" "Chat, Projects, Automation, Learning, Capabilities"
  require_contains "$readme documents the public alpha channel" "$readme" "ghcr.io/vivien83/captain-agent-os:alpha"
  require_contains "$readme links the immutable current release" "$readme" "https://github.com/Vivien83/captain/releases/tag/v0.1.0-alpha.2"
  require_contains "$readme pins the immutable current image" "$readme" "ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.2"
  require_contains "$readme pins the prerelease installer" "$readme" "releases/download/v0.1.0-alpha.2/install.sh"
  require_contains "$readme opens the Control root" "$readme" 'http://127.0.0.1:50051/'
  require_not_contains "$readme does not use GitHub latest for a prerelease" "$readme" "releases/latest/download/install.sh"
  require_not_contains "$readme does not require a registry token" "$readme" "GHCR_TOKEN"
  require_not_contains "$readme has no private candidate version" "$readme" "0.1.0-dev.2026-07-13a"
  require_contains "$readme documents local release publication" "$readme" "scripts/publish-release-local.sh"
  require_contains "$readme documents deterministic Docker embeddings" "$readme" "FastEmbed"
done
require_contains "English README discloses missing notarization" README.md "not Apple-notarized"
require_contains "French README discloses missing notarization" README.fr.md "ne sont pas notarisés"
require_contains "Spanish README discloses missing notarization" README.es.md "no están notarizados"
require_contains "Chinese README discloses missing notarization" README.zh.md "尚未经过 Apple notarization"
require_contains "English README documents proactive Codex discovery" README.md "an hourly refresh surfaces newly listed models"
require_contains "French README documents proactive Codex discovery" README.fr.md "une actualisation horaire signale les nouveaux modèles"
require_contains "Spanish README documents proactive Codex discovery" README.es.md "una actualización cada hora muestra los modelos nuevos"
require_contains "Chinese README documents proactive Codex discovery" README.zh.md "每小时刷新一次目录"
require_contains "current runtime changelog entry is pinned" docs/captain-tools/runtime-changelog.md "### 0.1.0-alpha.2"
require_contains "public changelog entry is pinned" CHANGELOG.md "## [0.1.0-alpha.2] - 2026-07-14"
require_contains "reviewed current alpha release notes exist" docs/releases/v0.1.0-alpha.2.md "# Captain 0.1.0-alpha.2"
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
require_contains "runtime pins Codex catalog protocol" crates/captain-runtime/src/model_catalog_codex.rs 'CODEX_CATALOG_CLIENT_VERSION: &str = "1.0.0"'
require_contains "browser docs pin same-model visual analysis" docs/captain-tools/browser.md "same active model"
require_contains "browser docs reject a secondary Vision agent" docs/captain-tools/browser.md "does not call a separate Vision agent"
require_contains "browser docs pin capture-only semantics" docs/captain-tools/browser.md "pixels are not injected into the model context"
require_contains "DOC2 pins native same-model images" docs/DOCS_STATUS.md "Images and prompted browser screenshots stay on the active conversation model"
require_contains "runtime validates active-model image support" crates/captain-kernel/src/capability_routing.rs "ensure_active_model_supports"
require_contains "runtime discloses no hidden image delegation" crates/captain-kernel/src/capability_routing.rs "did not send the image to another agent or provider"
require_not_contains "runtime has no automatic Vision-agent manifest" crates/captain-kernel/src/capability_routing.rs "build_vision_agent_manifest"
require_not_contains "runtime has no hidden image spawn path" crates/captain-kernel/src/capability_routing.rs "SpawnAndDelegate"
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
