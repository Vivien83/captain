#!/usr/bin/env bash
# Reproducible Captain Core Excellence gates by user-facing surface.

set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd -P)
SURFACE="all"
DRY_RUN=0
LIST_ONLY=0

usage() {
  cat <<'USAGE'
Usage:
  scripts/core-surface-gates.sh [--surface <name>|--all] [--dry-run]
  scripts/core-surface-gates.sh --list

Surfaces:
  chat
  projects
  automation
  learning
  capabilities
  settings-status

This is a thin, versioned map over scripts/gate.sh. It keeps repeated release
checks reproducible without replacing the live smoke gates:
  scripts/user-flow-smoke.sh --channel telegram
  scripts/hermes-vs-captain-benchmark.sh
  scripts/release-readiness.sh
USAGE
}

surfaces() {
  printf '%s\n' \
    chat \
    projects \
    automation \
    learning \
    capabilities \
    settings-status
}

quote_command() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
}

run_gate() {
  local surface="$1"
  shift
  printf '\n== surface: %s\n' "$surface"
  quote_command "$ROOT_DIR/scripts/gate.sh" "$@"
  if [[ "$DRY_RUN" -eq 0 ]]; then
    "$ROOT_DIR/scripts/gate.sh" "$@"
  fi
}

gate_chat() {
  run_gate chat \
    --check captain-cli \
    --check captain-api \
    --check captain-runtime \
    --check captain-channels \
    --test captain-cli chat_runner \
    --test captain-cli slash_standalone \
    --test captain-cli stream_lifecycle \
    --test captain-api ws_terminal \
    --test captain-api streaming_channels \
    --test captain-api channel_bridge \
    --test captain-runtime agent_loop_completion \
    --test captain-runtime agent_loop_tool_flow \
    --test captain-channels command_format \
    --test captain-channels inbound_queue \
    --script-check scripts/tui-smoke.sh \
    --script-check scripts/user-flow-smoke.sh
}

gate_projects() {
  run_gate projects \
    --check captain-cli \
    --check captain-api \
    --check captain-runtime \
    --check captain-memory \
    --test captain-api project_runtime \
    --test captain-api project_tool_request \
    --test captain-api project_answer \
    --test captain-cli project \
    --test captain-cli status_project_attention \
    --test captain-runtime project_runtime \
    --test captain-memory project \
    --test captain-memory project_task \
    --test captain-memory project_checkpoint \
    --test captain-memory milestone \
    --script-check scripts/project-runtime-restart-smoke.sh
}

gate_automation() {
  run_gate automation \
    --check captain-cli \
    --check captain-api \
    --check captain-kernel \
    --check captain-runtime \
    --test captain-kernel cron \
    --test captain-kernel kernel_cron_runtime \
    --test captain-kernel kernel_trigger_runtime \
    --test captain-api cron_routes \
    --test captain-api schedule_routes \
    --test captain-api webhook_routes \
    --test captain-cli cron \
    --test captain-runtime schedule \
    --test captain-runtime depth_schedule \
    --script-check scripts/excellence-smoke.sh
}

gate_learning() {
  run_gate learning \
    --check captain-cli \
    --check captain-api \
    --check captain-kernel \
    --check captain-runtime \
    --check captain-memory \
    --check captain-channels \
    --check captain-skills \
    --test captain-runtime skill_proposal_approval_tests \
    --test captain-runtime skill_diff \
    --test captain-runtime skill_writer \
    --test captain-kernel kernel_handle_memory \
    --test captain-api skill_routes \
    --test captain-api learning_routes \
    --test captain-cli skill \
    --test captain-cli learning_fetch \
    --test captain-memory learning_review \
    --test captain-memory skill_patterns \
    --test captain-memory skill_proposals \
    --test captain-channels command_review \
    --test captain-channels telegram_callbacks
}

gate_capabilities() {
  run_gate capabilities \
    --check captain-cli \
    --check captain-api \
    --check captain-kernel \
    --check captain-runtime \
    --test captain-runtime capability_search \
    --test captain-runtime tool_search \
    --test captain-runtime registry \
    --test captain-api server_capability_routes \
    --test captain-api tool_routes \
    --test captain-kernel capabilities \
    --test captain-kernel capability_routing \
    --test captain-cli models \
    --test captain-cli resource_status
}

gate_settings_status() {
  run_gate settings-status \
    --check captain-cli \
    --check captain-api \
    --check captain-runtime \
    --check captain-types \
    --test captain-cli status \
    --test captain-cli status_health \
    --test captain-cli status_verbose \
    --test captain-cli doctor \
    --test captain-api status_routes \
    --test captain-api status_runtime_health \
    --test captain-api config_routes \
    --test captain-runtime model_catalog \
    --test captain-types config \
    --script-check scripts/debug-noise-audit.sh \
    --script-check scripts/docs-global-audit.sh \
    --script-check scripts/docs-release-audit.sh \
    --script-check scripts/frozen-surface-audit.sh \
    --script-check scripts/release-readiness.sh
}

run_surface() {
  case "$1" in
    chat) gate_chat ;;
    projects) gate_projects ;;
    automation) gate_automation ;;
    learning) gate_learning ;;
    capabilities) gate_capabilities ;;
    settings-status) gate_settings_status ;;
    *)
      printf 'unknown surface: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --surface)
      if [[ $# -lt 2 || -z "${2:-}" ]]; then
        printf 'missing value after --surface\n' >&2
        exit 2
      fi
      SURFACE="$2"
      shift 2
      ;;
    --all)
      SURFACE="all"
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --list)
      LIST_ONLY=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      printf 'unknown argument: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "$LIST_ONLY" -eq 1 ]]; then
  surfaces
  exit 0
fi

cd "$ROOT_DIR"

if [[ "$SURFACE" == "all" ]]; then
  while IFS= read -r surface; do
    run_surface "$surface"
  done < <(surfaces)
else
  run_surface "$SURFACE"
fi
