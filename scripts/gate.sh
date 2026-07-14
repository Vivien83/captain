#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/gate.sh --check <package> --test <package> <filter> [--test <package> <filter> ...] [--script-check <path> ...] [--run-script <path> ...]

Runs the tranche gate:
  cargo fmt --all --check
  cargo check -p <package>...
  cargo test -p <package> <filter>...
  bash -n <path>...
  execute <path>...
  git diff --check
  git diff --cached --check

Examples:
  scripts/gate.sh --check captain-kernel --check captain-api \
    --test captain-kernel kernel_streaming_runtime \
    --test captain-kernel streaming
USAGE
}

checks=()
tests=()
script_checks=()
run_scripts=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --check)
      if [[ $# -lt 2 ]]; then
        echo "missing package after --check" >&2
        usage >&2
        exit 2
      fi
      checks+=("$2")
      shift 2
      ;;
    --test)
      if [[ $# -lt 3 ]]; then
        echo "missing package/filter after --test" >&2
        usage >&2
        exit 2
      fi
      tests+=("$2"$'\t'"$3")
      shift 3
      ;;
    --script-check)
      if [[ $# -lt 2 ]]; then
        echo "missing path after --script-check" >&2
        usage >&2
        exit 2
      fi
      script_checks+=("$2")
      shift 2
      ;;
    --run-script)
      if [[ $# -lt 2 ]]; then
        echo "missing path after --run-script" >&2
        usage >&2
        exit 2
      fi
      run_scripts+=("$2")
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ ${#checks[@]} -eq 0 && ${#tests[@]} -eq 0 && ${#script_checks[@]} -eq 0 && ${#run_scripts[@]} -eq 0 ]]; then
  echo "at least one --check, --test, --script-check or --run-script is required" >&2
  usage >&2
  exit 2
fi

run() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
  "$@"
}

run cargo fmt --all --check

if [[ ${#checks[@]} -gt 0 ]]; then
  for package in "${checks[@]}"; do
    run cargo check -p "$package"
  done
fi

if [[ ${#tests[@]} -gt 0 ]]; then
  for spec in "${tests[@]}"; do
    package="${spec%%$'\t'*}"
    filter="${spec#*$'\t'}"
    run cargo test -p "$package" "$filter"
  done
fi

if [[ ${#script_checks[@]} -gt 0 ]]; then
  for script in "${script_checks[@]}"; do
    run bash -n "$script"
  done
fi

if [[ ${#run_scripts[@]} -gt 0 ]]; then
  for script in "${run_scripts[@]}"; do
    run "$script"
  done
fi

run git diff --check
run git diff --cached --check
