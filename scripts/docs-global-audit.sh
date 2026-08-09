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
  docs/performance-budgets.md
  docs/SKILL_LEARNING_V2.md
  docs/CAPTAIN_FORGE_CAPSPEC.md
  docs/architecture.md
  docs/security.md
  docs/evidence/DEPENDENCY_SECURITY_BASELINE_2026-07-30.md
  docs/evidence/RELEASE_SUPPLY_CHAIN_BASELINE_2026-07-30.md
  docs/agent-templates.md
  docs/workflows.md
  docs/captain-tools/browser.md
  docs/deployment/github-vps-install.md
  docs/deployment/vps-web-terminal.md
  docs/releases/v0.1.0-alpha.12.md
  docs/releases/v0.1.0-alpha.11.md
  docs/releases/v0.1.0-alpha.10.md
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

require_missing_file() {
  local file="$1"
  if [ ! -e "$file" ]; then
    pass "removed file stays absent: $file"
  else
    fail "removed file unexpectedly present: $file"
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

require_pretest_not_contains() {
  local label="$1"
  local file="$2"
  local pattern="$3"
  if sed '/#\[cfg(test)\]/,$d' "$file" | grep -Fq "$pattern"; then
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
  rg -v -i 'frozen|disabled|absent|unavailable|rejected|fail(s|ed)? before|compat|historical|outside the active|not active|retained|migration' "$raw" >"$out" || true
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
require_contains "DOC2 pins Control XSS smoke" docs/DOCS_STATUS.md "scripts/control-xss-smoke.mjs"
require_contains "DOC2 classifies performance budgets" docs/DOCS_STATUS.md "docs/performance-budgets.md"
require_contains "DOC2 pins Control performance smoke" docs/DOCS_STATUS.md "scripts/control-chat-performance-smoke.mjs"
require_contains "DOC2 pins compaction terminal smoke" docs/DOCS_STATUS.md "scripts/compaction-progress-terminal-smoke.mjs"
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
scan_contract_banned \
  "current contract docs do not mislabel the audit chain as Merkle" \
  'Merkle'

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
require_contains "security docs classify content guards honestly" docs/security.md 'heuristic pattern classification, not information-flow'
require_contains "security docs disclose missing provenance propagation" docs/security.md 'do **not**'
require_not_contains "security docs make no active taint-tracking claim" docs/security.md 'Information Flow Taint Tracking'
require_not_contains "API docs make no active taint-tracking claim" docs/api-reference.md '"taint_tracking"'
require_contains "API status exposes heuristic content guards" crates/captain-api/src/security_routes.rs '"content_pattern_guards"'
require_contains "API status denies provenance tracking" crates/captain-api/src/security_routes.rs '"provenance_tracking": false'
require_not_contains "runtime security guard has no taint abstraction" crates/captain-runtime/src/tools/security.rs 'Taint'
require_not_contains "shared types no longer export taint tracking" crates/captain-types/src/lib.rs 'pub mod taint'
require_contains "DOC2 classifies the versioned audit hash chain" docs/DOCS_STATUS.md 'Alpha 10 Audit Hash Chain Contract'
require_contains "DOC2 pins injective audit field encoding" docs/DOCS_STATUS.md '`u64` big-endian byte length'
require_contains "DOC2 pins the active audit epoch partition" docs/DOCS_STATUS.md 'Every sequence at or after the active epoch start must belong'
require_contains "architecture documents immutable recovery epochs" docs/architecture.md '`ChainRecovery`'
require_contains "security docs pin audit persistence before memory" docs/security.md 'SQLite insertion completes before'
require_contains "API reference exposes authenticated audit integrity" docs/api-reference.md '"active_epoch_valid": true'
require_contains "runtime changelog exposes audit recovery" docs/captain-tools/runtime-changelog.md '`ChainRecovery` epoch'
require_not_contains "runtime changelog does not claim Merkle audit" docs/captain-tools/runtime-changelog.md 'Merkle'
require_contains "audit record propagates persistence failures" crates/captain-runtime/src/audit.rs 'pub fn record('
require_contains "audit record returns a typed result" crates/captain-runtime/src/audit.rs ') -> Result<String, AuditError>'
require_contains "audit v2 length-prefixes every field" crates/captain-runtime/src/audit_chain.rs 'hash_length_prefixed(&mut hasher, field)'
require_contains "audit length prefix is u64 big-endian" crates/captain-runtime/src/audit_chain.rs '(field.len() as u64).to_be_bytes()'
require_contains "audit schema adds recovery epochs" crates/captain-memory/src/migration.rs 'CREATE TABLE IF NOT EXISTS audit_epochs'
require_contains "health detail exposes audit integrity" crates/captain-api/src/health_routes.rs '"audit": audit'
require_not_contains "coordination router has no audit repair handler" crates/captain-api/src/server_coordination_routes.rs 'axum::routing::post(routes::audit_repair)'
require_not_contains "audit routes export no repair handler" crates/captain-api/src/audit_routes.rs 'pub async fn audit_repair'
require_contains "security reconciliation marks F6 remediated" docs/evidence/SECURITY_AUDIT_RECONCILIATION_2026-07-29.md '| F6 | Remediated (T5) |'
require_contains "DOC2 defines the exact host execution posture" docs/DOCS_STATUS.md "Alpha 10 Host Execution Posture"
require_contains "DOC2 denies host OS isolation" docs/DOCS_STATUS.md '`os_isolation: false`'
require_contains "architecture names the host subprocess boundary honestly" docs/architecture.md "### Host Subprocess Boundary"
require_not_contains "architecture does not label host execution as subprocess isolation" docs/architecture.md "### Subprocess Isolation"
require_contains "security guide classifies dangerous-command recognition honestly" docs/security.md '`normalized_lexical_heuristic`, not proof'
require_contains "tool docs deny an OS sandbox for host execution" docs/captain-tools/shell-process.md "do not create a namespace, seccomp, Landlock, chroot, or"
require_contains "API docs expose the host execution backend" docs/api-reference.md '"backend": "host_process"'
require_contains "API docs deny host subprocess OS isolation" docs/api-reference.md '"subprocess_os_isolation": false'
require_not_contains "API docs have no stale subprocess isolation claim" docs/api-reference.md '"subprocess_isolation": true'
require_contains "security API exposes environment scrubbing" crates/captain-api/src/security_routes.rs '"subprocess_environment_scrub": true'
require_contains "security API denies host OS isolation" crates/captain-api/src/security_routes.rs '"subprocess_os_isolation": false'
require_not_contains "security API has no stale subprocess isolation claim" crates/captain-api/src/security_routes.rs '"subprocess_isolation": true'
require_contains "types expose the exact host execution backend" crates/captain-types/src/config/execution.rs 'HOST_EXECUTION_BACKEND: &str = "host_process"'
require_contains "types expose the exact host isolation level" crates/captain-types/src/config/execution.rs 'HOST_EXECUTION_ISOLATION_LEVEL: &str = "environment_scrub"'
require_contains "types deny host operating-system isolation" crates/captain-types/src/config/execution.rs 'HOST_EXECUTION_OS_ISOLATED: bool = false'
require_contains "types classify the dangerous-command guard honestly" crates/captain-types/src/config/execution.rs 'DANGEROUS_COMMAND_GUARD_LEVEL: &str = "normalized_lexical_heuristic"'
require_contains "types expose personal workstation execution profile" crates/captain-types/src/config/execution.rs 'PersonalWorkstation'
require_contains "types expose remote operator execution profile" crates/captain-types/src/config/execution.rs 'RemoteOperator'
require_contains "types expose untrusted execution profile" crates/captain-types/src/config/execution.rs 'UntrustedExecution'
require_contains "types make allowlist the structural execution default" crates/captain-types/src/config/execution.rs 'fn exec_policy_default_is_allowlisted_and_explicitly_non_isolated'
require_contains "daemon and agent execution policies intersect" crates/captain-runtime/src/tool_runner.rs 'Some(agent.intersect(global))'
require_contains "process supervisor requires an execution permit" crates/captain-runtime/src/process_manager.rs 'start_in_dir_with_permit'
require_contains "process start uses the guarded process surface" crates/captain-runtime/src/tools/process_ops.rs 'ExecSurface::ProcessTool'
require_contains "Docker execution denies host fallback" crates/captain-runtime/src/tools/docker_ops.rs 'Captain will not fall back to host execution.'
require_contains "API distinguishes configured execution mode" crates/captain-api/src/security_routes.rs 'configured_policy_mode'
require_contains "Control exposes the execution profile" crates/captain-api/static/js/app/status_model.mjs "profile: stringAt(execution.profile, 'unknown')"
require_contains "configuration guide documents explicit-only routing" docs/configuration.md 'Docker routing is always `explicit_only`'
require_contains "tool docs include process_start in guarded execution" docs/captain-tools/shell-process.md 'WASM host execution, and `process_start`'
require_contains "security reconciliation records HARDEN11 F7 closure" docs/evidence/SECURITY_AUDIT_RECONCILIATION_2026-07-29.md '## HARDEN11 follow-up — 2026-07-30'
require_contains "reference config uses the allowlist default" captain.toml.example 'mode = "allowlist"'
require_contains "reference config uses safe critical mode" captain.toml.example 'critical_mode = "safe"'
require_contains "quick init records trusted workstation profile" crates/captain-cli/src/commands/init.rs 'profile = "personal_workstation"'
require_contains "quick init writes safe critical mode" crates/captain-cli/src/commands/init.rs 'critical_mode = "safe"'
require_contains "setup wizard records trusted workstation profile" crates/captain-cli/src/tui/screens/wizard.rs 'profile = "personal_workstation"'
require_contains "setup wizard writes safe critical mode" crates/captain-cli/src/tui/screens/wizard.rs 'critical_mode = "safe"'
require_contains "first-use wizard records trusted workstation profile" crates/captain-cli/src/tui/screens/init_wizard.rs 'profile = "personal_workstation"'
require_contains "first-use wizard writes safe critical mode" crates/captain-cli/src/tui/screens/init_wizard.rs 'critical_mode = "safe"'
require_contains "security reconciliation marks F7 remediated" docs/evidence/SECURITY_AUDIT_RECONCILIATION_2026-07-29.md '| F7 | Remediated (T8) |'
require_contains "skills expose a typed remote marketplace freeze" crates/captain-skills/src/lib.rs 'RemoteMarketplaceFrozen'
require_contains "legacy marketplace client fails before I/O" crates/captain-skills/src/marketplace.rs 'require_remote_marketplace_access()?;'
require_contains "legacy ClawHub client fails before I/O" crates/captain-skills/src/clawhub.rs 'require_remote_marketplace_access()?;'
require_pretest_not_contains "active skill router omits remote install" crates/captain-api/src/server_skill_routes.rs '"/api/skills/install"'
require_pretest_not_contains "active skill router omits marketplace search" crates/captain-api/src/server_skill_routes.rs '"/api/marketplace/search"'
require_pretest_not_contains "active skill router omits ClawHub" crates/captain-api/src/server_skill_routes.rs '"/api/clawhub/'
require_missing_file "crates/captain-api/src/clawhub_routes.rs"
require_not_contains "CLI skill install has no remote fallback client" crates/captain-cli/src/commands/skill.rs 'MarketplaceClient::new'
require_contains "CLI skill install requires a local directory" crates/captain-cli/src/commands/skill.rs 'is not a local skill directory'
require_not_contains "TUI skills hide ClawHub actions" crates/captain-cli/src/tui/screens/skills.rs 'ClawHub'
require_contains "prompt scan exposes advisory assurance" crates/captain-skills/src/verify.rs 'AdvisoryHeuristic'
require_contains "doctor reports the advisory assurance" crates/captain-cli/src/commands/doctor/environment.rs '"assurance": "advisory_heuristic"'
require_contains "security guide denies phrase-scan proof" docs/security.md 'A finding is not proof of an attack'
require_contains "skill guide documents the fail-before-I/O boundary" docs/captain-tools/skill.md 'before network or filesystem access'
require_not_contains "API guide omits removed remote install endpoint" docs/api-reference.md '### POST /api/skills/install'
require_contains "security reconciliation marks F8 remediated" docs/evidence/SECURITY_AUDIT_RECONCILIATION_2026-07-29.md '| F8 | Remediated (T9) |'
require_contains "security reconciliation marks F9 remediated" docs/evidence/SECURITY_AUDIT_RECONCILIATION_2026-07-29.md '| F9 | Remediated (T9) |'
require_file "docs/evidence/DEPENDENCY_SECURITY_BASELINE_2026-07-30.md"
require_contains "public security policy exposes the unfiltered dependency audit" SECURITY.md 'a second audit without exceptions'
require_contains "security guide pins the FastEmbed ORT ABI" docs/security.md '`fastembed 5.13.2` remains pinned to `ort 2.0.0-rc.11`'
require_contains "dependency evidence distinguishes reviewed from absent advisories" docs/evidence/DEPENDENCY_SECURITY_BASELINE_2026-07-30.md 'no unreviewed vulnerability'
require_contains "dependency gate runs an unfiltered audit" scripts/dependency-audit.sh 'cargo audit --json --no-fetch --file "$ROOT_DIR/Cargo.lock"'
require_contains "dependency gate rejects RSA packages" scripts/dependency-audit.sh 'versions("rsa") == []'
require_contains "dependency gate rejects RSA russh features" scripts/dependency-audit.sh '== ["aws-lc-rs", "flate2"]'
require_contains "dependency gate removes legacy IMAP parser" scripts/dependency-audit.sh 'versions("lexical-core") == []'
require_contains "dependency gate pins the IMAP client" scripts/dependency-audit.sh 'versions("imap") == ["3.0.0-alpha.15"]'
require_contains "dependency gate pins the IMAP parser" scripts/dependency-audit.sh 'versions("imap-proto") == ["0.16.7"]'
require_contains "dependency evidence explains the exact IMAP prerelease" docs/evidence/DEPENDENCY_SECURITY_BASELINE_2026-07-30.md 'prerelease preserves Captain'
require_contains "runtime changelog exposes the IMAP modernization" docs/captain-tools/runtime-changelog.md 'maintained `imap-proto 0.16.7` parser'
require_contains "email IMAP keeps implicit TLS on custom ports" crates/captain-channels/src/email.rs '.mode(imap::ConnectionMode::Tls)'
require_contains "workspace SSH dependency disables default features" Cargo.toml 'russh = { version = "=0.62.4", default-features = false'
require_not_contains "workspace SSH key features omit RSA" Cargo.toml '"ed25519", "rsa"'
if [ "$INTERNAL_DOCS_PRESENT" = "1" ]; then
  require_contains "maintainer SSH guide exposes the RSA security boundary" docs/ssh-setup.md 'RSA est'
else
  require_contains "public security guide exposes the RSA security boundary" docs/security.md 'RSA SSH private keys and RSA-only server host keys are intentionally'
fi
require_not_contains "public Compose omits frozen Slack credentials" docker-compose.yml 'SLACK_BOT_TOKEN'
require_not_contains "configuration guide omits frozen Slack setup" docs/configuration.md '[channels.slack]'
require_not_contains "provider guide has no copied model catalog" docs/providers.md '**Available Models:**'
require_not_contains "provider guide has no volatile price table" docs/providers.md '$/1M'
require_contains "skill guide requires complete local source review" docs/skill-development.md 'Review the complete local skill'
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
require_contains "docs README links performance budgets" docs/README.md "performance-budgets.md"
require_contains "docs index links performance budgets" docs/INDEX.md "performance-budgets.md"
require_contains "performance contract pins the visual frame" docs/performance-budgets.md "at most 34 ms"
require_contains "performance contract forbids dropped deltas" docs/performance-budgets.md "No text delta may be dropped"
require_contains "performance contract pins the real browser smoke" docs/performance-budgets.md "scripts/control-chat-performance-smoke.mjs"
require_not_contains "docs navigation does not advertise frozen migration" docs/README.md 'MIGRATION.md'
for readme in README.md README.fr.md README.es.md README.zh.md; do
  require_contains "$readme pins the six operational hubs" "$readme" "Chat, Projects, Automation, Learning, Capabilities"
  require_contains "$readme documents the public alpha channel" "$readme" "ghcr.io/vivien83/captain-agent-os:alpha"
  require_contains "$readme links the immutable release" "$readme" "https://github.com/Vivien83/captain/releases/tag/v0.1.0-alpha.12"
  require_contains "$readme pins the immutable release image" "$readme" "ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.12"
  require_contains "$readme pins the release installer" "$readme" "releases/download/v0.1.0-alpha.12/install.sh"
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
require_contains "CLI documents macOS post-update vault authorization" docs/cli-reference.md 'Keychain authorization dialog for the `captain-vault` item'
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
require_contains "release candidate runtime changelog entry is pinned" docs/captain-tools/runtime-changelog.md "### 0.1.0-alpha.12"
require_contains "release candidate public changelog entry is pinned" CHANGELOG.md "## [0.1.0-alpha.12] - 2026-08-09"
require_contains "published alpha.12 release notes exist" docs/releases/v0.1.0-alpha.12.md "# Captain 0.1.0-alpha.12"
require_contains "historical alpha.11 release notes remain available" docs/releases/v0.1.0-alpha.11.md "# Captain 0.1.0-alpha.11"
require_contains "historical alpha.10 release notes remain available" docs/releases/v0.1.0-alpha.10.md "# Captain 0.1.0-alpha.10"
require_contains "historical alpha.9 release notes remain available" docs/releases/v0.1.0-alpha.9.md "# Captain 0.1.0-alpha.9"
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
require_contains "DOC2 identifies alpha.9 as earlier history" docs/DOCS_STATUS.md '`v0.1.0-alpha.9` is an earlier public prerelease'
require_contains "DOC2 identifies the alpha.10 earlier release" docs/DOCS_STATUS.md '`v0.1.0-alpha.10` is an earlier public prerelease'
require_contains "DOC2 pins the alpha.10 host asset count" docs/DOCS_STATUS.md 'exactly 22 files'
require_contains "DOC2 records the published alpha.10 provenance" docs/DOCS_STATUS.md "48f898a9e4d38e8b8c7627644b66e22076a39364"
require_contains "DOC2 records the published alpha.10 tag object" docs/DOCS_STATUS.md "b58f7561d0014228cc523b1770b5c411b017ef52"
require_contains "DOC2 records the published alpha.10 multi-arch digest" docs/DOCS_STATUS.md "sha256:c54d1319b5173ca55540dc69e0f965a31b51cdfccb497ca77882882a16b4e477"
require_contains "agent changelog records the published alpha.10 multi-arch digest" docs/captain-tools/runtime-changelog.md "sha256:c54d1319b5173ca55540dc69e0f965a31b51cdfccb497ca77882882a16b4e477"
require_contains "alpha.10 notes pin live source provenance" docs/releases/v0.1.0-alpha.10.md "48f898a9e4d38e8b8c7627644b66e22076a39364"
require_contains "alpha.10 notes pin live OCI provenance" docs/releases/v0.1.0-alpha.10.md "sha256:c54d1319b5173ca55540dc69e0f965a31b51cdfccb497ca77882882a16b4e477"
require_contains "alpha.10 notes record zero hosted workflows" docs/releases/v0.1.0-alpha.10.md "GitHub Actions API returned zero runs"
require_not_contains "alpha.10 notes do not copy alpha.9 source provenance" docs/releases/v0.1.0-alpha.10.md "1248c5928dd4968b6ff7c62ef79a607fb8d94348"
require_not_contains "alpha.10 notes do not copy alpha.9 OCI provenance" docs/releases/v0.1.0-alpha.10.md "sha256:b043ec5637551c2e238be15c32033ca693ecc2f765a470ba721a5986709fd692"
require_contains "DOC2 identifies the alpha.12 public release" docs/DOCS_STATUS.md '`v0.1.0-alpha.12` is the current public prerelease'
require_contains "DOC2 records the published alpha.12 provenance" docs/DOCS_STATUS.md 'cd7bf8ab5674b402d06e36bb1c4ae9b4a5ab16a2'
require_contains "DOC2 records the published alpha.12 tag object" docs/DOCS_STATUS.md '651a018593ea2d21af2e2a50d786d7f35654be9d'
require_contains "DOC2 records the published alpha.12 multi-arch digest" docs/DOCS_STATUS.md 'sha256:5626ba43317b6341f123a5041f6d1e473db0486217c9b9912f3fd5bb41e45afa'
require_contains "agent changelog records the published alpha.12 digest" docs/captain-tools/runtime-changelog.md 'sha256:5626ba43317b6341f123a5041f6d1e473db0486217c9b9912f3fd5bb41e45afa'
require_contains "alpha.12 notes pin live source provenance" docs/releases/v0.1.0-alpha.12.md 'cd7bf8ab5674b402d06e36bb1c4ae9b4a5ab16a2'
require_contains "alpha.12 notes pin live OCI provenance" docs/releases/v0.1.0-alpha.12.md 'sha256:5626ba43317b6341f123a5041f6d1e473db0486217c9b9912f3fd5bb41e45afa'
require_contains "alpha.12 notes record zero hosted workflows" docs/releases/v0.1.0-alpha.12.md 'GitHub Actions API returned zero runs'
require_not_contains "alpha.12 notes do not copy alpha.11 source provenance" docs/releases/v0.1.0-alpha.12.md 'cd7f580a5e89ea77852468bc4fad9875f00dce61'
require_not_contains "alpha.12 notes do not copy alpha.11 OCI provenance" docs/releases/v0.1.0-alpha.12.md 'sha256:7dbed4eff2d57e88a0fcc33d343f942454d3a1b29ea933102d050c8d7a9b1192'
require_contains "DOC2 identifies the alpha.11 previous release" docs/DOCS_STATUS.md '`v0.1.0-alpha.11` is the previous public prerelease'
require_contains "DOC2 records the published alpha.11 provenance" docs/DOCS_STATUS.md 'cd7f580a5e89ea77852468bc4fad9875f00dce61'
require_contains "DOC2 records the published alpha.11 tag object" docs/DOCS_STATUS.md 'fafc41e33386ec370f3da17d24650e370d46af4e'
require_contains "DOC2 records the published alpha.11 multi-arch digest" docs/DOCS_STATUS.md 'sha256:7dbed4eff2d57e88a0fcc33d343f942454d3a1b29ea933102d050c8d7a9b1192'
require_contains "agent changelog records the published alpha.11 digest" docs/captain-tools/runtime-changelog.md 'sha256:7dbed4eff2d57e88a0fcc33d343f942454d3a1b29ea933102d050c8d7a9b1192'
require_contains "alpha.11 notes pin live source provenance" docs/releases/v0.1.0-alpha.11.md 'cd7f580a5e89ea77852468bc4fad9875f00dce61'
require_contains "alpha.11 notes pin live OCI provenance" docs/releases/v0.1.0-alpha.11.md 'sha256:7dbed4eff2d57e88a0fcc33d343f942454d3a1b29ea933102d050c8d7a9b1192'
require_contains "alpha.11 notes record zero hosted workflows" docs/releases/v0.1.0-alpha.11.md 'GitHub Actions API returned zero runs'
require_not_contains "alpha.11 notes do not copy alpha.10 source provenance" docs/releases/v0.1.0-alpha.11.md "48f898a9e4d38e8b8c7627644b66e22076a39364"
require_not_contains "alpha.11 notes do not copy alpha.10 OCI provenance" docs/releases/v0.1.0-alpha.11.md "sha256:c54d1319b5173ca55540dc69e0f965a31b51cdfccb497ca77882882a16b4e477"
require_contains "alpha.12 notes expose durable Live Runs" docs/releases/v0.1.0-alpha.12.md "## Durable Live Runs"
require_contains "alpha.12 notes expose grounded research" docs/releases/v0.1.0-alpha.12.md "## Evidence-grounded Web research"
require_contains "alpha.12 notes expose immutable artifacts" docs/releases/v0.1.0-alpha.12.md "## Immutable artifacts"
require_contains "alpha.12 notes expose managed VPS domains" docs/releases/v0.1.0-alpha.12.md "## Managed VPS domains and readiness"
require_contains "alpha.12 notes pin the authorized destination boundary" docs/releases/v0.1.0-alpha.12.md "selective metadata and redacted tail are not sent"
require_contains "DOC2 identifies the Alpha 12 published contract" docs/DOCS_STATUS.md "## Alpha 12 Published Contract"
require_contains "DOC2 retains the alpha.8 public history" docs/DOCS_STATUS.md '`v0.1.0-alpha.8` is an earlier public prerelease'
require_contains "DOC2 retains the alpha.7 source provenance" docs/DOCS_STATUS.md "dc2f64603eff708a8eab5735121cfc1a2d39386f"
require_contains "DOC2 retains the alpha.7 multi-arch digest" docs/DOCS_STATUS.md "sha256:e49e1ad02d6a65742343aaf7abcd1c4fcfd277dab605d3d284830f03c7d42354"
require_contains "agent changelog retains the alpha.7 multi-arch digest" docs/captain-tools/runtime-changelog.md "sha256:e49e1ad02d6a65742343aaf7abcd1c4fcfd277dab605d3d284830f03c7d42354"
require_contains "DOC2 retains the alpha.7 public history" docs/DOCS_STATUS.md '`v0.1.0-alpha.7` is an earlier public prerelease'
require_contains "DOC2 discloses the alpha.9 memory opt-out limitation" docs/DOCS_STATUS.md "core agent-loop finalizer to write one local episodic interaction"
require_contains "DOC2 records the alpha.10 finalizer fix" docs/DOCS_STATUS.md 'The published `alpha.10` release closes that limitation'
require_contains "DOC2 discloses the alpha.8 memory opt-out limitation" docs/DOCS_STATUS.md "the core agent-loop finalizer still writes its local episodic interaction"
require_contains "memory docs disclose the alpha.8 finalizer limitation" docs/captain-tools/memory.md "agent-loop finalizer still stores one local episodic"
require_contains "memory docs expose the alpha.10 finalizer boundary" docs/captain-tools/memory.md 'The published `alpha.10` release closes that finalizer'
require_contains "runtime finalizer gates episodic capture on opt-out" crates/captain-runtime/src/agent_loop_finish.rs "memory_write_opt_out(user_message)"
require_contains "DOC2 defines the synchronized live budget contract" docs/DOCS_STATUS.md "Alpha 10 Live Budget Contract"
require_contains "DOC2 defines the guarded host execution posture" docs/DOCS_STATUS.md "Alpha 10 Host Execution Posture"
require_contains "API documents persist-before-publish budget updates" docs/api-reference.md "atomically persists that exact snapshot"
require_contains "architecture uses one live budget authority" docs/architecture.md 'dedicated `RwLock<BudgetConfig>`'
require_contains "runtime changelog exposes live budget enforcement" docs/captain-tools/runtime-changelog.md "global cost guard"
require_contains "kernel owns the synchronized budget update boundary" crates/captain-kernel/src/kernel_budget_config.rs "A persistence failure leaves the live state unchanged"
require_contains "API denies invalid shared-reference casting" crates/captain-api/src/lib.rs "#![deny(invalid_reference_casting)]"
require_not_contains "budget route has no pointer-derived config mutation" crates/captain-api/src/usage_budget_routes.rs "config_ptr"
require_not_contains "budget route has no raw mutable cast" crates/captain-api/src/usage_budget_routes.rs "as *mut"
require_contains "agent budget edits update the live scheduler" crates/captain-api/src/usage_budget_routes.rs "set_hourly_quota(agent_id, value)"
require_contains "security reconciliation marks F1 remediated" docs/evidence/SECURITY_AUDIT_RECONCILIATION_2026-07-29.md "| F1 | Remediated"
require_contains "DOC2 defines independent browser session signing" docs/DOCS_STATUS.md "Alpha 10 Browser Session Contract"
require_contains "DOC2 defines deny-by-default API authentication" docs/DOCS_STATUS.md "Alpha 10 API Authentication Contract"
require_contains "configuration forbids credential-derived session signing" docs/configuration.md "never the daemon API key or"
require_contains "API documents session epoch invalidation" docs/api-reference.md 'Changing the password through setup or `web_credentials_update` advances that'
require_contains "architecture documents fail-closed signing reload" docs/architecture.md "Live auth reload fails closed"
require_contains "runtime changelog exposes independent session signing" docs/captain-tools/runtime-changelog.md "one independent 32-byte random key per"
require_contains "auth config declares managed signing key" crates/captain-types/src/config/auth.rs "Captain-managed base64 encoding of a 32-byte session signing key"
require_contains "kernel provisions signing state before runtime boot" crates/captain-kernel/src/kernel_boot_foundations.rs "ensure_session_signing_state"
require_contains "session auth derives only from managed secret" crates/captain-api/src/session_auth.rs "derive_session_secret(&self.auth.session_secret)"
require_contains "session tokens enforce credential epoch" crates/captain-api/src/session_auth.rs "session_epoch != expected_session_epoch"
require_contains "config secret registry covers session signing key" crates/captain-types/src/config/secret_fields.rs '["auth", "session_secret"]'
require_contains "raw config display redacts managed auth state" crates/captain-api/src/config_routes.rs "redact_auth_secrets"
require_contains "web credential rotation advances session epoch" crates/captain-runtime/src/tools/web_credentials_ops.rs "increment_session_epoch_for_password_change"
require_contains "agent config reads redact auth signing material" crates/captain-runtime/src/tools/config_ops.rs "is_secret_auth_config_path"
require_contains "kernel config writes reject managed auth state" crates/captain-kernel/src/kernel_handle_config.rs "is_managed_auth_config_path"
require_contains "DOC2 records Argon2id browser passwords" docs/DOCS_STATUS.md "salted Argon2id PHC strings"
require_contains "configuration documents secure cookie policy" docs/configuration.md 'session_cookie_secure = "auto"'
require_contains "configuration documents fail-closed loopback opt-out" docs/configuration.md 'allow_unauthenticated_loopback = false'
require_contains "auth defaults reject credentialless loopback" crates/captain-types/src/config/auth.rs "allow_unauthenticated_loopback: false"
require_contains "auth boot migrates only explicit legacy opt-out" crates/captain-types/src/config/auth.rs 'unwrap_or(matches!(persisted_enabled, Some(false)))'
require_contains "API credentialless mode requires actual loopback" crates/captain-api/src/middleware.rs "&& client_is_loopback"
require_contains "setup closes credentialless loopback mode" crates/captain-cli/src/commands/setup_access.rs '"allow_unauthenticated_loopback".to_string()'
require_contains "status distinguishes unconfigured auth" crates/captain-api/src/status_payload.rs '(false, false, false) => "unconfigured"'
require_contains "doctor reports explicit loopback auth opt-out" crates/captain-cli/src/commands/doctor/environment.rs '"unauthenticated_loopback"'
require_contains "API documents one-time realtime ticket" docs/api-reference.md "30-second single-use ticket"
require_contains "security reconciliation closes F3" docs/evidence/SECURITY_AUDIT_RECONCILIATION_2026-07-29.md "| F3 | Remediated (T6 + T7)"
require_contains "web password hashing uses Argon2id" crates/captain-types/src/config/auth.rs "Argon2::default()"
require_contains "legacy login migrates password hash" crates/captain-api/src/web_auth_routes.rs "migrate_legacy_password_hash"
require_contains "login limiter tracks IP and username" crates/captain-api/src/web_auth_security.rs "by_user"
require_contains "login limiter has a bounded saturation backoff" crates/captain-api/src/web_auth_security.rs "LOGIN_SATURATION_BACKOFF"
require_contains "login limiter never evicts an active block under pressure" crates/captain-api/src/web_auth_security.rs "capacity_pressure_never_evicts_an_active_login_block"
require_contains "public deployment docs require an upstream login limiter" docs/deployment/vps-web-terminal.md "Add an upstream login request limit"
require_contains "realtime tickets are consumed once" crates/captain-api/src/web_auth_security.rs "state.tickets.remove(ticket)"
require_contains "browser chat requests realtime ticket" crates/captain-api/static/js/app/api.js "api.realtimeTicket(path)"
require_contains "API auth has one explicit public allowlist" crates/captain-api/src/middleware.rs "const PUBLIC_ALLOWLIST"
require_contains "API auth matrix is source reviewed" crates/captain-api/src/middleware_auth_matrix_tests.rs "public_allowlist_is_exactly_the_reviewed_matrix"
require_not_contains "stale public-read policy test is removed" crates/captain-api/src/middleware.rs "public_endpoint_policy_keeps_mutations_private"
require_contains "API docs enumerate private operational surfaces" docs/api-reference.md "Every other route is private"
require_contains "security docs explain private OAuth decision" docs/security.md "GitHub Copilot OAuth flow require"
require_contains "Control emits centralized unauthorized signal" crates/captain-api/static/js/app/api.js "captain:unauthorized"
require_contains "Control consumes centralized unauthorized signal" crates/captain-api/static/js/app/main.js "captain:unauthorized"
require_contains "security reconciliation closes F4" docs/evidence/SECURITY_AUDIT_RECONCILIATION_2026-07-29.md "| F4 | Remediated (T3)"
require_contains "DOC2 defines fail-closed browser origins" docs/DOCS_STATUS.md "Alpha 10 Browser Origin Contract"
require_contains "security policy supports only alpha.12" SECURITY.md '| 0.1.0-alpha.12 | :white_check_mark: |'
require_contains "security policy retires alpha.11" SECURITY.md '| 0.1.0-alpha.11 | :x: |'
require_contains "security policy pins the alpha.12 deployment boundary" SECURITY.md 'Captain `0.1.0-alpha.12` is an early-access release.'
require_contains "configuration exposes exact API origins" docs/configuration.md 'allowed_origins = ["https://console.example.com"]'
require_contains "reference config exposes the API origin section" captain.toml.example "[api]"
require_not_contains "API CORS policy has no wildcard helper" crates/captain-api/src/request_origin_security.rs "tower_http::cors::Any"
require_contains "API mounts exact Host validation" crates/captain-api/src/server.rs "request_origin_security::validate_host"
require_contains "API origin changes require restart" crates/captain-kernel/src/config_reload.rs "API allowed origins changed"
require_contains "security reconciliation closes F5" docs/evidence/SECURITY_AUDIT_RECONCILIATION_2026-07-29.md "| F5 | Remediated (T4)"
require_not_contains "security policy has no stale key-dependent CORS claim" SECURITY.md "Restricted to localhost when no API key configured"
require_not_contains "API middleware has no query credential field" crates/captain-api/src/middleware.rs "query_token"
require_not_contains "agent websocket has no query token fallback" crates/captain-api/src/ws.rs 'strip_prefix("token=")'
require_not_contains "terminal websocket has no query token fallback" crates/captain-api/src/ws_terminal.rs 'query_param(uri, "token")'
require_contains "CLI config access classifies managed auth state" crates/captain-cli/src/commands/config.rs "is_secret_auth_config_path"
require_contains "security reconciliation marks F3 closed" docs/evidence/SECURITY_AUDIT_RECONCILIATION_2026-07-29.md "| F3 | Remediated (T6 + T7)"
require_contains "historical alpha.2 release notes remain available" docs/releases/v0.1.0-alpha.2.md "# Captain 0.1.0-alpha.2"
require_contains "historical alpha release notes remain available" docs/releases/v0.1.0-alpha.1.md "# Captain 0.1.0-alpha.1"
require_contains "runtime changelog marks 07-12b as published" docs/captain-tools/runtime-changelog.md "is the published release that includes the aligned"
require_not_contains "runtime changelog has no stale 07-12b candidate claim" docs/captain-tools/runtime-changelog.md 'aligned candidate is `0.1.0-dev.2026-07-12b`'
require_contains "runtime changelog supersedes old WEB1 claim" docs/captain-tools/runtime-changelog.md "superseded by this entry"
require_contains "README documents the seven-question first-use interview" README.md "seven-question onboarding interview"
require_contains "French README documents the seven-question first-use interview" README.fr.md "sept questions"
require_contains "Spanish README documents the seven-question first-use interview" README.es.md "siete preguntas"
require_contains "Chinese README documents the seven-question first-use interview" README.zh.md "七个问题"
require_contains "DOC2 classifies non-blocking first-use suggestions" docs/DOCS_STATUS.md "Alpha 10 First-Use Interaction Contract"
require_contains "DOC2 classifies authoritative credentials" docs/DOCS_STATUS.md "Alpha 10 Credential Contract"
require_contains "DOC2 classifies learning visibility" docs/DOCS_STATUS.md "Alpha 10 Learning Visibility Contract"
require_contains "API documents non-blocking suggested replies" docs/api-reference.md '"type": "suggested_replies"'
require_contains "daemon SSE documents suggested replies" docs/api-reference.md '| `suggested_replies` | Non-blocking choices.'
require_contains "Telegram adapter docs distinguish suggested replies from ask_user" docs/channel-adapters.md "Non-blocking suggested replies"
require_contains "agent channel docs distinguish suggested replies from ask_user" docs/captain-tools/channel.md "Non-blocking suggested replies"
require_contains "runtime changelog exposes first-use suggested replies" docs/captain-tools/runtime-changelog.md "seven-question first-use interview"
require_contains "first-use notification opt-out controls native silent mode" crates/captain-kernel/src/kernel_first_use.rs '"silent_mode"'
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
require_contains "deployment pins cross-surface session recovery after SIGKILL" docs/DEPLOY.md 'activates a detached cross-surface session'
require_contains "deployment pins audit chain continuity after SIGKILL" docs/DEPLOY.md 'same valid hash-chain epoch'
require_contains "deployment pins Live Run reconciliation after SIGKILL" docs/DEPLOY.md 'in-flight Live Run and its partial owner-only capture'
require_contains "power-loss smoke restores an exact session" scripts/persistence-power-loss-smoke.sh '"recovered session can be activated"'
require_contains "power-loss smoke preserves the audit tip" scripts/persistence-power-loss-smoke.sh '"pre-crash audit tip remains in the recovered chain"'
require_contains "power-loss smoke uses fail-closed daemon auth" scripts/persistence-power-loss-smoke.sh 'Authorization: Bearer $API_KEY'
require_contains "power-loss smoke seeds a durable in-flight Live Run" scripts/persistence-power-loss-smoke.sh 'seed_inflight_tool_run'
require_contains "power-loss smoke certifies redacted partial evidence" scripts/persistence-power-loss-smoke.sh '"partial tool-run evidence is recovered and redacted after SIGKILL"'
require_contains "public changelog records session recovery after power loss" CHANGELOG.md 'detached cross-surface session can'
require_contains "public changelog records Live Run recovery after power loss" CHANGELOG.md 'in-flight Live Run is also restored as `interrupted`'
require_contains "runtime changelog records audit continuity after power loss" docs/captain-tools/runtime-changelog.md 'pre-crash audit tip still belongs to the same healthy'
require_contains "runtime changelog records interrupted Live Run recovery" docs/captain-tools/runtime-changelog.md 'boot restores it as `interrupted`'
require_contains "DOC2 pins the full ALPHA12 power-loss proof" docs/DOCS_STATUS.md 'retain the pre-crash audit tip in the same healthy epoch'
require_contains "DOC2 pins Live Run recovery in the ALPHA12 power-loss proof" docs/DOCS_STATUS.md 'synthetic in-flight Live Run to `interrupted`'
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
require_contains "host installer normalizes a VPS domain before dependencies" scripts/install.sh 'prepare_vps_domain'
require_contains "host installer refuses occupied proxy ports" scripts/install.sh 'Ports 80 or 443 are already in use'
require_contains "host installer validates Caddy before activation" scripts/install.sh 'validate --config "$caddy_root" --adapter caddyfile'
require_contains "host installer restores failed Caddy activation" scripts/install.sh 'The previous configuration was restored'
require_contains "host installer verifies public Captain Web" scripts/install.sh 'Captain Web verified end to end'
require_contains "host installer distinguishes browser and API credentials" scripts/install.sh 'API bearer key: for CLI/API clients; it is not pasted into the browser login'
require_contains "VPS guide pins automatic browser session creation" docs/deployment/github-vps-install.md 'A successful browser login creates the'
require_contains "VPS guide pins the managed Alpha 12 installer" docs/deployment/github-vps-install.md 'The `v0.1.0-alpha.12` installer includes Captain'
require_contains "VPS docs pin managed-domain rollback" docs/deployment/github-vps-install.md 'restores the previous files if validation or activation fails'
require_not_contains "getting-started does not describe the domain rail as post-Alpha 11" docs/getting-started.md 'post-Alpha 11'
require_not_contains "deployment guide does not describe the domain rail as post-Alpha 11" docs/DEPLOY.md 'post-Alpha 11'
require_not_contains "Web terminal guide does not describe the domain rail as post-Alpha 11" docs/deployment/vps-web-terminal.md 'post-Alpha 11'
require_not_contains "troubleshooting does not describe the domain rail as post-Alpha 11" docs/troubleshooting.md 'post-Alpha 11'
require_not_contains "configuration does not describe approval core as post-Alpha 11" docs/configuration.md 'post-Alpha 11'
require_contains "VPS domain regression test exists" scripts/install-vps-domain-test.sh 'managed VPS domain installer is validated in isolation'
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
require_contains "CLI documents versioned catalog export" docs/cli-reference.md 'captain.session.export.v1'
require_contains "DOC2 pins non-activating catalog export" docs/DOCS_STATUS.md 'reads this global catalog'
require_contains "runtime changelog exposes catalog JSONL" docs/captain-tools/runtime-changelog.md 'exports the complete durable session'
require_contains "session export writes through durable filesystem" crates/captain-cli/src/commands/session_export.rs 'captain_types::durable_fs::atomic_write'
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
require_contains "API reference exposes authenticated artifact routes" docs/api-reference.md '## Artifact Endpoints'
require_contains "API reference pins artifact payload verification" docs/api-reference.md 'verify both its byte count and'
require_contains "API reference pins SVG download-only" docs/api-reference.md 'SVG or unknown/active format'
require_contains "API reference pins immutable artifact retention" docs/api-reference.md 'There is intentionally no artifact mutation or deletion endpoint'
require_contains "security docs pin artifact CSP sandbox" docs/security.md 'response CSP starts with `sandbox`'
require_contains "security docs pin exact artifact preview path" docs/security.md '/api/artifacts/{uuid}/versions/{positive-version}/preview'
require_contains "DOC2 classifies ALPHA12 artifact source contract" docs/DOCS_STATUS.md '/api/status.artifacts'
require_contains "runtime changelog exposes artifact operator surfaces" docs/captain-tools/runtime-changelog.md 'Authenticated operator endpoints list artifacts and versions'
require_contains "Control deployment guide pins artifact drawer without a seventh hub" docs/deployment/vps-web-terminal.md 'It is not a seventh hub'
require_contains "architecture pins sandboxed artifact drawer" docs/architecture.md 'Supported previews run in an empty-sandbox, no-referrer iframe'
require_contains "DOC2 pins artifact drawer browser smoke" docs/DOCS_STATUS.md '344 px foldable layouts'
require_contains "runtime changelog exposes artifact drawer" docs/captain-tools/runtime-changelog.md 'global `Fichiers produits` drawer'
require_contains "CLI reference exposes TUI artifact overlay" docs/cli-reference.md '`/artifacts` opens the global read-only'
require_contains "CLI reference keeps artifact preview in Control" docs/cli-reference.md 'sandboxed preview and verified download remain in authenticated Control Web'
require_contains "VPS terminal documents bounded artifact summary" docs/deployment/vps-web-terminal.md 'Its `/artifacts` command'
require_contains "architecture pins TUI artifact metadata-only contract" docs/architecture.md 'inventory, exact versions, integrity metadata and provenance but never renders'
require_contains "DOC2 pins standalone artifact interception" docs/DOCS_STATUS.md '`/artifacts` before the model and return at most twelve payload-free rows'
require_contains "runtime changelog exposes TUI artifact overlay" docs/captain-tools/runtime-changelog.md 'Full Ratatui Chat now opens the same immutable inventory through `/artifacts`'
require_contains "full TUI implements artifact overlay command" crates/captain-cli/src/tui/mod.rs 'fn open_artifacts(&mut self)'
require_contains "standalone TUI intercepts artifact command" crates/captain-cli/src/tui/chat_runner.rs 'fn handle_artifacts_slash(&mut self'
require_contains "TUI artifact screen is metadata-only" crates/captain-cli/src/tui/screens/artifacts.rs 'active content is never rendered in the terminal'
require_contains "TUI retains six primary hubs" crates/captain-cli/src/tui/mod.rs 'assert_eq!(TABS.len(), 6);'
require_contains "Control embeds the artifact drawer" crates/captain-api/src/webchat.rs '"components/ArtifactDrawer.js"'
require_contains "core surface gate runs artifact drawer smoke" scripts/core-surface-gates.sh 'scripts/control-artifact-drawer-smoke.mjs'
require_contains "artifact API mounts authenticated inventory" crates/captain-api/src/server_artifact_routes.rs '"/api/artifacts"'
require_contains "artifact API reads verified bytes" crates/captain-memory/src/artifacts.rs 'pub fn read_verified_payload'
require_not_contains "artifact API exposes no destructive route" crates/captain-api/src/server_artifact_routes.rs '.delete('
require_contains "API reference exposes authenticated Live Runs routes" docs/api-reference.md '## Live Run Endpoints'
require_contains "API reference pins bounded Live Runs tail" docs/api-reference.md 'capped at 32 KiB'
require_contains "security docs pin strict Live Runs cancellation" docs/security.md 'run is active'
require_contains "architecture omits Live Runs disk authority" docs/architecture.md 'does not inherit its full internal representation'
require_contains "DOC2 classifies authenticated Live Runs API" docs/DOCS_STATUS.md 'Authenticated `/api/tool-runs` operator routes'
require_contains "runtime changelog exposes Live Runs operator API" docs/captain-tools/runtime-changelog.md 'Authenticated `/api/tool-runs` endpoints'
require_contains "API reference exposes Control Live Runs drawer" docs/api-reference.md '**Live Runs** drawer'
require_contains "architecture pins text-only Control Live Runs tail" docs/architecture.md 'as text nodes, never HTML'
require_contains "DOC2 pins Control Live Runs browser smoke" docs/DOCS_STATUS.md 'redacted/XSS-safe tail'
require_contains "VPS guide exposes authenticated Live Runs drawer" docs/deployment/vps-web-terminal.md 'global **Live Runs** drawer'
require_contains "runtime changelog exposes Control Live Runs drawer" docs/captain-tools/runtime-changelog.md 'Live Runs operator drawer'
require_contains "Control embeds the Live Runs drawer" crates/captain-api/src/webchat.rs '"components/LiveRunsDrawer.js"'
require_contains "Control consumes private Live Runs inventory" crates/captain-api/static/js/app/api.js "request(withQuery('/api/tool-runs'"
require_contains "Control renders Live Runs tail as text" crates/captain-api/static/js/app/components/LiveRunsDrawer.js 'html`<pre>${tail.content}</pre>`'
require_contains "Control gates Live Runs cancellation on cancellable metadata" crates/captain-api/static/js/app/components/LiveRunsDrawer.js '!selected.cancellable'
require_contains "core surface gate runs Live Runs drawer smoke" scripts/core-surface-gates.sh 'scripts/control-live-runs-smoke.mjs'
require_contains "Live Runs API mounts authenticated inventory" crates/captain-api/src/server_observability_routes.rs '"/api/tool-runs"'
require_contains "Live Runs surfaces share selective projection" crates/captain-runtime/src/tool_run_operator.rs 'pub struct OperatorToolRun'
require_contains "Live Runs projection never exports result preview" crates/captain-runtime/src/tool_run_operator.rs 'result_available: snapshot.result_preview.is_some()'
require_contains "Live Runs surfaces share fail-closed tail" crates/captain-runtime/src/tool_run_operator.rs 'pub fn operator_tail('
require_contains "Live Runs API uses kernel cancellation authority" crates/captain-api/src/tool_run_routes.rs '.operator_cancel_tool_run(ToolRunOperatorSurface::Api, &run_id)'
require_contains "Live Runs kernel identifies API and TUI cancellation" crates/captain-kernel/src/kernel_handle_tool_runs.rs 'Self::Tui => "operator:tui"'
require_contains "Live Runs strict cancellation is atomic" crates/captain-runtime/src/tool_runs.rs 'CancellationPolicy::ActiveCancellableOnly'
require_contains "Live Runs API is pinned private" crates/captain-api/src/middleware_auth_matrix_tests.rs '"/api/tool-runs/toolrun-01234567-89ab-cdef-0123-456789abcdef/tail"'
require_not_contains "Live Runs API does not serialize internal snapshot" crates/captain-api/src/tool_run_routes.rs 'serde_json::to_value(snapshot)'
require_contains "CLI reference exposes global Live Runs overlay" docs/cli-reference.md '`/runs` opens the global **Live Runs** overlay'
require_contains "VPS terminal keeps Live Runs summary bounded" docs/deployment/vps-web-terminal.md 'Its `/runs` command is also intercepted before the model'
require_contains "DOC2 pins TUI stale-tail protection" docs/DOCS_STATUS.md 'Stale tail responses cannot replace the'
require_contains "runtime changelog exposes TUI Live Runs overlay" docs/captain-tools/runtime-changelog.md 'Full Ratatui now opens the same inventory through `/runs`'
require_contains "full TUI implements Live Runs overlay command" crates/captain-cli/src/tui/mod.rs 'fn open_live_runs(&mut self)'
require_contains "standalone TUI intercepts Live Runs command" crates/captain-cli/src/tui/chat_runner.rs 'fn handle_tool_runs_slash(&mut self'
require_contains "TUI Live Runs shares operator projection" crates/captain-cli/src/tui/screens/live_runs.rs 'OperatorToolRunTail'
require_contains "TUI Live Runs rejects stale tail responses" crates/captain-cli/src/tui/screens/live_runs.rs 'self.loading_tail_for.as_deref() != Some(run_id)'
require_contains "TUI Live Runs requires explicit confirmation" crates/captain-cli/src/tui/screens/live_runs.rs "KeyCode::Char('y')"
require_contains "kernel proves TUI Live Runs cancellation actor" crates/captain-kernel/src/kernel_handle_tool_runs.rs 'tui_cancellation_aborts_the_task_and_uses_the_fixed_audit_actor'
require_contains "English README exposes TUI Live Runs" README.md '<code>/runs</code>'
require_contains "French README exposes TUI Live Runs" README.fr.md '<code>/runs</code>'
require_contains "Spanish README exposes TUI Live Runs" README.es.md '<code>/runs</code>'
require_contains "Chinese README exposes TUI Live Runs" README.zh.md '<code>/runs</code>'
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
require_contains "API documents exact approval rules" docs/api-reference.md '## Approval Endpoints'
require_contains "API documents durable approval revocation" docs/api-reference.md 'DELETE /api/approvals/rules/{id}'
require_contains "security docs pin approval action digests" docs/security.md 'domain-separated BLAKE3 digest'
require_contains "approval binding uses complete serialized tool input" crates/captain-runtime/src/tools/dispatch_guard.rs 'serde_json::to_vec(input)'
require_contains "approval request carries digest separately from display preview" crates/captain-kernel/src/kernel_handle_approval.rs 'action_digest: action_digest.to_string()'
require_contains "security docs separate preview from approval binding" docs/security.md 'human-facing preview is a separate bounded field'
require_contains "CLI documents scoped approval keys" docs/cli-reference.md '`R`, `D`, and `X` open a'
require_contains "Telegram docs expose six approval scopes" docs/channel-adapters.md 'six explicit'
require_contains "DOC2 pins cross-surface approval semantics" docs/DOCS_STATUS.md 'Tool approvals are one shared operator contract'
require_contains "approval rules use crash-safe writes" crates/captain-kernel/src/approval_rules.rs 'durable_fs::atomic_write'
require_contains "approval rules persist no raw action field" crates/captain-types/src/approval.rs 'pub action_digest: String'
require_contains "approval API exposes rule revocation" crates/captain-api/src/server_governance_routes.rs '"/api/approvals/rules/{id}"'
require_contains "Telegram approval keyboard exposes durable deny" crates/captain-channels/src/telegram_callbacks.rs 'approval:deny_always:'
require_contains "approval suggestions are opt-in by default" crates/captain-types/src/approval_suggestions.rs 'enabled: false'
require_contains "approval suggestions exclude High and Critical risk" crates/captain-types/src/approval_suggestions.rs 'RiskLevel::Low | RiskLevel::Medium'
require_contains "approval suggestion consent creates an exact allow rule" crates/captain-kernel/src/approval.rs 'effect: ApprovalRuleEffect::Allow'
require_contains "approval suggestion boot reconciles committed rules" crates/captain-kernel/src/approval.rs 'remove_covered_bindings'
require_pretest_not_contains "approval suggestion store persists no display description" crates/captain-kernel/src/approval_suggestions.rs 'action_summary'
require_pretest_not_contains "approval suggestion store persists no raw description" crates/captain-kernel/src/approval_suggestions.rs 'description'
require_not_contains "approval suggestions do not shape the LLM prompt" crates/captain-kernel/src/kernel_llm_prompt.rs 'ApprovalSuggestion'
require_contains "configuration documents approval suggestions as core-only" docs/configuration.md 'This checkpoint is an internal ALPHA12 contract.'
require_contains "security docs pin suggestion no-authority boundary" docs/security.md 'A pending suggestion has no authority'
require_contains "DOC2 classifies approval suggestions as core-only" docs/DOCS_STATUS.md 'core-only approval-suggestion store'
require_contains "runtime changelog avoids claiming suggestion surfaces" docs/captain-tools/runtime-changelog.md 'controls are not yet claimed by this checkpoint'
require_not_contains "CapSpec contract has no stale open-matrix claim" docs/CAPTAIN_FORGE_CAPSPEC.md 'broad real certification matrix is still open'
require_not_contains "runtime changelog has no stale open CapSpec gate" docs/captain-tools/runtime-changelog.md 'broad real certification matrix remains required'
require_contains "configuration documents external secret registry" docs/configuration.md '$CAPTAIN_HOME/secret-sources.toml'
require_contains "configuration pins authoritative external mappings" docs/configuration.md 'external mapping is **authoritative**'
require_contains "configuration exposes redacted source status CLI" docs/configuration.md 'captain vault sources --json'
require_contains "CLI documents external source status" docs/cli-reference.md 'captain vault sources [--json]'
require_contains "CLI documents native vault key storage" docs/cli-reference.md 'macOS Keychain, Windows Credential Manager, or Linux Secret Service'
require_contains "CLI documents headless vault key contract" docs/cli-reference.md 'headless and CI deployments must provide'
require_contains "security docs pin verified native vault storage" docs/security.md 'Every new key is read back and compared'
require_contains "security docs pin fail-closed legacy migration" docs/security.md 'native/legacy mismatch refuses both automatic deletion and overwrite'
require_contains "vault uses the real platform keyring crate" crates/captain-extensions/src/vault_keyring.rs 'keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)'
require_not_contains "vault never prints generated master keys" crates/captain-extensions/src/vault_keyring.rs 'eprintln!'
require_contains "CLI channel setup uses canonical credential resolver" docs/cli-reference.md 'Saves local tokens through the canonical credential resolver'
require_contains "channel docs distinguish Gmail OAuth from IMAP SMTP" docs/channel-adapters.md 'Captain exposes two deliberately separate email rails'
require_contains "configuration documents named Email accounts" docs/configuration.md '[[channels.email.accounts]]'
require_contains "API documents channel form schema authority" docs/api-reference.md 'authoritative guided-client schema'
require_contains "API documents named Email live probes" docs/api-reference.md 'Email probe performs real IMAP login/folder checks'
require_contains "CLI documents named Email tests" docs/cli-reference.md 'captain channel test email:work'
require_not_contains "CLI docs expose no dead channel enable command" docs/cli-reference.md 'captain channel enable <CHANNEL>'
require_not_contains "CLI parser exposes no dead channel enable variant" crates/captain-cli/src/cli_args_config.rs 'Enable {'
require_not_contains "CLI parser exposes no dead channel disable variant" crates/captain-cli/src/cli_args_config.rs 'Disable {'
require_contains "tool docs put external sources first" docs/captain-tools/config-secret.md '`~/.captain/secret-sources.toml` file mapping'
require_contains "tool docs pin external fail-closed behavior" docs/captain-tools/config-secret.md 'authoritative and fail-closed'
require_contains "security docs block mounted source targets" docs/security.md 'all configured source targets join the kernel'
require_contains "DOC2 pins one credential contract" docs/DOCS_STATUS.md 'one credential resolution contract'
require_contains "runtime loads versioned external sources" crates/captain-extensions/src/external_secret_sources.rs 'SECRET_SOURCES_SCHEMA_VERSION: u32 = 1'
require_contains "external secret read buffer is zeroized" crates/captain-extensions/src/external_secret_sources.rs 'let text = Zeroizing::new('
require_contains "kernel blocks external source registry" crates/captain-kernel/src/kernel_workspace_security.rs 'captain_home.join("secret-sources.toml")'
require_contains "kernel blocks configured external source targets" crates/captain-kernel/src/kernel.rs '.external_source_paths()'
require_contains "generic file tools enforce external targets for ordinary agents" crates/captain-runtime/src/tools/file_paths.rs 'blocklist_applies_to_ordinary_agents_without_extra_roots'
require_contains "CLI central resolver unlocks vault and external sources" crates/captain-cli/src/cli_support.rs 'CredentialResolver::from_home(vault, home)'
require_contains "all CLI local secret mutations refuse external keys" crates/captain-cli/src/dotenv.rs 'local_secret_mutations_refuse_externally_managed_keys'
require_contains "CLI channel setup uses production resolver" crates/captain-cli/src/commands/channel.rs 'production_credential_resolver_at(home)'
require_contains "CLI Signal setup writes runtime phone field" crates/captain-cli/src/commands/channel.rs 'phone_number = {phone}'
require_not_contains "CLI Signal setup has no stale phone env field" crates/captain-cli/src/commands/channel.rs 'phone_env ='
require_contains "signed webhooks use kernel credential resolver" crates/captain-api/src/event_webhooks.rs 'state.kernel.resolve_credential(key)'
require_contains "signed webhooks test missing-secret fail-closed" crates/captain-api/src/event_webhooks.rs 'configured_webhook_signature_fails_closed_when_secret_is_unavailable'
require_contains "API status uses the credential resolver for channel readiness" crates/captain-api/src/status_routes.rs 'state.kernel.resolve_credential(key)'
require_contains "shared API secret writes refuse external keys" crates/captain-api/src/secret_env.rs 'shared_secret_writer_refuses_externally_managed_keys'
require_contains "agent API ingress uses the credential resolver" crates/captain-api/src/agent_api_routes.rs 'validate_agent_api_token_with(headers, agent_id'
require_contains "agent API egress queue uses the credential resolver" crates/captain-api/src/agent_api_egress_queue.rs 'kernel.resolve_credential(key)'
require_contains "agent API external credentials are not disclosed" crates/captain-types/src/agent_api.rs 'Captain will not disclose or overwrite it'
require_contains "agent API external ingress has a non-disclosure proof" crates/captain-types/src/agent_api.rs 'existing_ingress_report_never_discloses_the_external_token'
require_contains "agent API queues transient external callback-source failures" crates/captain-api/src/agent_api_egress.rs 'unavailable_external_callback_is_visible_and_retryable'
require_contains "provider drivers cannot reuse stale env after external-source failure" crates/captain-runtime/src/drivers/mod.rs 'explicit_empty_key_blocks_stale_environment_fallback'
require_contains "dynamic agent drivers preserve authoritative external-source failure" crates/captain-kernel/src/kernel_driver_support.rs 'unavailable_external_provider_key_blocks_dynamic_driver_env_fallback'
require_contains "release monitor reports unavailable authoritative token" crates/captain-kernel/src/release_updates_tests.rs 'release_monitor_reports_unavailable_authoritative_token'
require_contains "CLI updater reports unavailable authoritative token" crates/captain-cli/src/commands/update.rs 'updater_reports_unavailable_authoritative_github_token'
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
require_file scripts/control-chat-performance-test.mjs
require_file scripts/control-chat-performance-smoke.mjs
require_file scripts/control-xss-smoke.mjs
require_file scripts/compaction-progress-terminal-smoke.mjs
require_contains "chat surface gate runs the deterministic Control audit" scripts/core-surface-gates.sh 'scripts/control-web-audit.sh'
require_contains "chat surface gate runs the browser XSS smoke" scripts/core-surface-gates.sh 'scripts/control-xss-smoke.mjs'
require_contains "chat surface gate runs the browser performance smoke" scripts/core-surface-gates.sh 'scripts/control-chat-performance-smoke.mjs'
require_contains "chat surface gate runs compaction terminal smoke" scripts/core-surface-gates.sh 'scripts/compaction-progress-terminal-smoke.mjs'
require_contains "browser CSP denies inline and evaluated script authority" crates/captain-api/src/middleware.rs "script-src 'self'; script-src-attr 'none'; style-src"
require_pretest_not_contains "browser CSP has no evaluated script authority" crates/captain-api/src/middleware.rs "'unsafe-eval'"
require_contains "security middleware emits the reviewed CSP" crates/captain-api/src/middleware.rs "security_header_middleware_emits_the_reviewed_csp"
require_not_contains "Desktop CSP has no evaluated script authority" crates/captain-desktop/tauri.conf.json "'unsafe-eval'"
require_not_contains "Control HTML no longer needs an import map" crates/captain-api/static/app_body.html 'type="importmap"'
require_contains "Control HTML loads its module as a same-origin asset" crates/captain-api/static/app_body.html 'src="/assets/app/main.js"'
require_contains "browser assets enforce external script tags" crates/captain-api/src/webchat.rs "every_browser_script_is_external_and_import_maps_are_absent"
require_contains "Markdown uses a fixed passive tag allowlist" crates/captain-api/static/js/app/components/Markdown.js "const MARKDOWN_TAGS"
require_contains "XSS smoke covers Markdown, tools and sessions" scripts/control-xss-smoke.mjs "Markdown, tool output, and session labels stay inert"
require_contains "DOC2 classifies compaction progress" docs/DOCS_STATUS.md "Alpha 10 Compaction Progress Contract"
require_contains "API documents typed compaction WebSocket progress" docs/api-reference.md '"type": "compaction_progress"'
require_contains "API documents daemon compaction SSE" docs/api-reference.md "event: compaction_progress"
require_contains "architecture pins compaction restart recovery" docs/architecture.md "SQLite schema v35"
require_contains "runtime changelog exposes truthful compaction progress" docs/captain-tools/runtime-changelog.md "opaque model calls remain visibly indeterminate"
require_contains "public changelog records compaction restart recovery" CHANGELOG.md "startup closes operations left by an earlier runtime instance"
require_file scripts/release-workflow-audit.sh
require_file scripts/publish-release-local.sh
require_file scripts/release-provenance.sh
require_file scripts/release-provenance-test.sh
require_file docs/release-provenance.md
require_file scripts/github-governance.sh
require_file scripts/github-discoverability.sh
require_file scripts/local-pr-gate.sh
require_file scripts/local-pr-gate-worker.sh
require_file scripts/local-pr-vm-bootstrap.sh
require_file scripts/local-pr-portal.sh
require_file scripts/local-pr-gate-test.sh
require_file scripts/local-pr-lima-smoke.sh
require_file docs/repository-governance.md
require_contains "docs index exposes release provenance" docs/INDEX.md "Release Provenance"
require_contains "docs README exposes release provenance" docs/README.md "Release provenance"
require_contains "docs index exposes repository governance" docs/INDEX.md "Repository Governance"
require_contains "repository governance documents the local required status" docs/repository-governance.md 'captain/local-pr-gate'
require_contains "repository governance documents disposable Lima execution" docs/repository-governance.md 'disposable plain-mode Lima VM'
require_contains "repository governance documents the portal service" docs/repository-governance.md 'scripts/local-pr-portal.sh --install-launchd'
require_contains "repository governance documents exact-base controller verification" docs/repository-governance.md 'exact protected-base SHA'
require_contains "repository governance documents root-owned toolchain and policies" docs/repository-governance.md 'root-owned, non-writable paths'
require_contains "repository governance documents immutable audit snapshots" docs/repository-governance.md 'immutable snapshots'
require_contains "repository governance applies versioned protection" docs/repository-governance.md 'scripts/github-governance.sh --apply'
require_contains "repository governance applies versioned discovery metadata" docs/repository-governance.md 'scripts/github-discoverability.sh --apply'
if [ "$SITE_PRESENT" = "1" ]; then
  require_contains "site deployment documents IndexNow notification" docs/deployment/launch-site.md 'api.indexnow.org/indexnow'
  require_contains "site deployment keeps Google indexing operator-owned" docs/deployment/launch-site.md 'Google Search Console'
fi
require_file "docs/evidence/RELEASE_SUPPLY_CHAIN_BASELINE_2026-07-30.md"
require_contains "release supply-chain evidence keeps remote policy honest" docs/evidence/RELEASE_SUPPLY_CHAIN_BASELINE_2026-07-30.md 'No document may claim the remote state is active'
require_contains "release supply-chain evidence pins sequential bundles" docs/evidence/RELEASE_SUPPLY_CHAIN_BASELINE_2026-07-30.md 'invoke each target separately'
require_file scripts/prepare-docker-embedding-cache.sh
require_file scripts/prepare-github-export.sh
require_file scripts/public-release-audit.sh
require_file scripts/public-boundary-guard.sh
require_file scripts/check-markdown-links.mjs
require_file scripts/public-export-smoke.sh
require_contains "release readiness audits the public source export" scripts/release-readiness.sh 'scripts/prepare-github-export.sh'
require_contains "DOC2 classifies local release provenance" docs/DOCS_STATUS.md "Alpha 10 Release Provenance"
require_contains "release provenance binds the public source" docs/release-provenance.md 'public Git source URI, commit, and tree'
require_contains "release provenance discloses missing independent signature" docs/release-provenance.md 'independently signed transparency-log attestation'
require_contains "release publisher builds Docker sequentially" scripts/publish-release-local.sh 'amd64_digest="$(build_and_push_architecture amd64)"'
require_contains "release publisher carries SLSA host provenance" scripts/publish-release-local.sh 'provenance.intoto.jsonl'
require_contains "DOC2 pins the public source audit" docs/DOCS_STATUS.md 'scripts/public-release-audit.sh'
require_contains "DOC2 pins the encoded public boundary guard" docs/DOCS_STATUS.md 'scripts/public-boundary-guard.sh'
require_contains "public boundary guard scans hidden files" scripts/public-boundary-guard.sh 'rg -n --hidden'
require_contains "public export smoke rejects a content probe" scripts/public-export-smoke.sh 'content guard accepted a blocked probe'
require_contains "public export smoke rejects a path probe" scripts/public-export-smoke.sh 'path guard accepted a blocked probe'
require_contains "public export smoke keeps content violations private" scripts/public-export-smoke.sh 'content guard disclosed a blocked probe'
require_contains "public export smoke keeps path violations private" scripts/public-export-smoke.sh 'path guard disclosed a blocked probe'
require_contains "DOC2 pins the public export smoke" docs/DOCS_STATUS.md 'scripts/public-export-smoke.sh'
require_file scripts/guarded-exec-audit.sh
require_file crates/captain-runtime/src/subprocess_env_scrub.rs
require_file crates/captain-runtime/src/subprocess_guard.rs
require_missing_file crates/captain-runtime/src/subprocess_sandbox.rs
require_missing_file crates/captain-runtime/src/subprocess_sandbox_tests.rs
require_contains "tranche gate enforces guarded execution audit" scripts/gate.sh 'scripts/guarded-exec-audit.sh'
require_contains "release readiness enforces guarded execution audit" scripts/release-readiness.sh 'scripts/guarded-exec-audit.sh'
require_contains "DOC2 classifies guarded execution boundary" docs/DOCS_STATUS.md "Alpha 10 Guarded Execution Contract"
require_contains "security docs name guarded execution source" docs/security.md 'captain-runtime/src/guarded_exec.rs'
require_not_contains "security docs have no subprocess sandbox overclaim" docs/security.md 'subprocess_sandbox'
require_not_contains "architecture has no subprocess sandbox overclaim" docs/architecture.md 'subprocess_sandbox'
require_contains "architecture documents content-bound approval permits" docs/architecture.md "content-bound permit"
require_contains "program permits use a versioned authorization domain" crates/captain-runtime/src/guarded_exec.rs 'captain.exec-permit.program.v1\0'
require_contains "program permits encode an explicit argument count" crates/captain-runtime/src/guarded_exec.rs 'program argument count must fit in u64'
require_contains "program permit boundary collisions have a regression proof" crates/captain-runtime/src/guarded_exec_tests.rs 'direct_program_authorization_encoding_is_injective'
require_contains "security docs pin injective program permit framing" docs/security.md 'big-endian `u64` argument count'
require_contains "public changelog records unified subprocess sinks" CHANGELOG.md "All agent-controlled subprocess sinks now share one guarded execution"
require_contains "shell tool docs name the shared guarded boundary" docs/captain-tools/shell-process.md '`guarded_exec` is the mandatory subprocess entry'
require_contains "shell tool docs pin exact safe environment" docs/captain-tools/shell-process.md '`PATH`, `HOME`, `TMPDIR`, `TMP`, `TEMP`, `LANG`'
require_contains "skill tool docs name the shared guarded boundary" docs/captain-tools/skill.md 'Skill checks and executions use `guarded_exec`'
require_contains "shell agent definition explains guarded execution" crates/captain-runtime/src/tools/shell_definitions.rs "frontière d'exécution gardée"
require_contains "skill agent definition explains guarded execution" crates/captain-runtime/src/tools/skill_definitions.rs "frontière d'exécution gardée"
require_contains "skill check definition explains guarded execution" crates/captain-runtime/src/tools/discovery.rs "frontière d'exécution gardée"

finish
