#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/gate.sh [--clippy-workspace] [--clippy <package> ...] [--clippy-lib <package> ...] [--clippy-bin <package> <bin> ...] [--test-workspace] --check <package> --test <package> <filter> [--test-ignored <package> <filter> ...] [--script-check <path> ...] [--run-script <path> ...]

Runs the tranche gate:
  cargo fmt --all --check
  scripts/guarded-exec-audit.sh
  cargo clippy --workspace -- -D warnings
  cargo clippy --all-targets -p <package> -- -D warnings
  cargo clippy --lib -p <package> -- -D warnings
  cargo clippy -p <package> --bin <bin> -- -D warnings
  cargo check -p <package>...
  cargo test --workspace
  cargo test -p <package> <filter>...
  cargo test -p <package> <filter> -- --ignored --test-threads=1
  bash -n <path>...
  execute <path>...
  git diff --check
  git diff --cached --check

Environment:
  CAPTAIN_GATE_CARGO_PROFILE  Cargo profile for clippy/check/test: dev (default) or release.

Examples:
  scripts/gate.sh --check captain-kernel --check captain-api \
    --test captain-kernel kernel_streaming_runtime \
    --test captain-kernel streaming
USAGE
}

checks=()
tests=()
ignored_tests=()
script_checks=()
run_scripts=()
clippy_packages=()
clippy_libs=()
clippy_bins=()
clippy_workspace=0
test_workspace=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --clippy-workspace)
      clippy_workspace=1
      shift
      ;;
    --clippy)
      if [[ $# -lt 2 ]]; then
        echo "missing package after --clippy" >&2
        usage >&2
        exit 2
      fi
      clippy_packages+=("$2")
      shift 2
      ;;
    --clippy-bin)
      if [[ $# -lt 3 ]]; then
        echo "missing package/bin after --clippy-bin" >&2
        usage >&2
        exit 2
      fi
      clippy_bins+=("$2"$'\t'"$3")
      shift 3
      ;;
    --clippy-lib)
      if [[ $# -lt 2 ]]; then
        echo "missing package after --clippy-lib" >&2
        usage >&2
        exit 2
      fi
      clippy_libs+=("$2")
      shift 2
      ;;
    --test-workspace)
      test_workspace=1
      shift
      ;;
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
    --test-ignored)
      if [[ $# -lt 3 ]]; then
        echo "missing package/filter after --test-ignored" >&2
        usage >&2
        exit 2
      fi
      ignored_tests+=("$2"$'\t'"$3")
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

if [[ $clippy_workspace -eq 0 && ${#clippy_packages[@]} -eq 0 && ${#clippy_libs[@]} -eq 0 && ${#clippy_bins[@]} -eq 0 && $test_workspace -eq 0 && ${#checks[@]} -eq 0 && ${#tests[@]} -eq 0 && ${#ignored_tests[@]} -eq 0 && ${#script_checks[@]} -eq 0 && ${#run_scripts[@]} -eq 0 ]]; then
  echo "at least one clippy, check, test, script-check or run-script gate is required" >&2
  usage >&2
  exit 2
fi

run() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
  "$@"
}

cargo_profile="${CAPTAIN_GATE_CARGO_PROFILE:-dev}"
case "$cargo_profile" in
  dev|release) ;;
  *)
    echo "CAPTAIN_GATE_CARGO_PROFILE must be dev or release" >&2
    exit 2
    ;;
esac

run cargo fmt --all --check
run scripts/guarded-exec-audit.sh

if [[ $clippy_workspace -eq 1 ]]; then
  if [[ "$cargo_profile" == "release" ]]; then
    run cargo clippy --release --workspace -- -D warnings
  else
    run cargo clippy --workspace -- -D warnings
  fi
fi

if [[ ${#clippy_packages[@]} -gt 0 ]]; then
  for package in "${clippy_packages[@]}"; do
    if [[ "$cargo_profile" == "release" ]]; then
      run cargo clippy --release --all-targets -p "$package" -- -D warnings
    else
      run cargo clippy --all-targets -p "$package" -- -D warnings
    fi
  done
fi

if [[ ${#clippy_libs[@]} -gt 0 ]]; then
  for package in "${clippy_libs[@]}"; do
    if [[ "$cargo_profile" == "release" ]]; then
      run cargo clippy --release --lib -p "$package" -- -D warnings
    else
      run cargo clippy --lib -p "$package" -- -D warnings
    fi
  done
fi

if [[ ${#clippy_bins[@]} -gt 0 ]]; then
  for spec in "${clippy_bins[@]}"; do
    package="${spec%%$'\t'*}"
    binary="${spec#*$'\t'}"
    if [[ "$cargo_profile" == "release" ]]; then
      run cargo clippy --release -p "$package" --bin "$binary" -- -D warnings
    else
      run cargo clippy -p "$package" --bin "$binary" -- -D warnings
    fi
  done
fi

if [[ ${#checks[@]} -gt 0 ]]; then
  for package in "${checks[@]}"; do
    if [[ "$cargo_profile" == "release" ]]; then
      run cargo check --release -p "$package"
    else
      run cargo check -p "$package"
    fi
  done
fi

if [[ $test_workspace -eq 1 ]]; then
  if [[ "$cargo_profile" == "release" ]]; then
    run cargo test --release --workspace
  else
    run cargo test --workspace
  fi
fi

if [[ ${#tests[@]} -gt 0 ]]; then
  for spec in "${tests[@]}"; do
    package="${spec%%$'\t'*}"
    filter="${spec#*$'\t'}"
    if [[ "$cargo_profile" == "release" ]]; then
      run cargo test --release -p "$package" "$filter"
    else
      run cargo test -p "$package" "$filter"
    fi
  done
fi

if [[ ${#ignored_tests[@]} -gt 0 ]]; then
  for spec in "${ignored_tests[@]}"; do
    package="${spec%%$'\t'*}"
    filter="${spec#*$'\t'}"
    if [[ "$cargo_profile" == "release" ]]; then
      run cargo test --release -p "$package" "$filter" -- --ignored --test-threads=1
    else
      run cargo test -p "$package" "$filter" -- --ignored --test-threads=1
    fi
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
