#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
TARGET=x86_64-pc-windows-msvc

if ! rustup target list --installed | grep -Fxq "$TARGET"; then
  printf 'required Rust target is not installed: %s\n' "$TARGET" >&2
  exit 1
fi

WORK_DIR=$(mktemp -d "${TMPDIR:-/tmp}/captain-node-windows-check.XXXXXX")
trap 'rm -rf -- "$WORK_DIR"' EXIT
mkdir -p "$WORK_DIR/src"
cp "$ROOT_DIR/crates/captain-node/src/native_service_control/windows.rs" \
  "$WORK_DIR/src/windows.rs"
cp "$ROOT_DIR/crates/captain-node/src/bin/captain-node/windows_service.rs" \
  "$WORK_DIR/src/service_runtime.rs"

cat > "$WORK_DIR/Cargo.toml" <<'EOF'
[package]
name = "captain-node-windows-service-check"
version = "0.0.0"
edition = "2021"
rust-version = "1.88"

[dependencies]
tokio = { version = "=1.50.0", features = ["rt-multi-thread"] }
tracing = "=0.1.44"
windows-service = "=0.8.1"
EOF

cat > "$WORK_DIR/src/lib.rs" <<'EOF'
#![allow(dead_code)]

extern crate self as captain_node;

use std::path::Path;

const NODE_WINDOWS_DISPLAY_NAME: &str = "Captain Node";
const NODE_WINDOWS_SERVICE: &str = "CaptainNode";

#[derive(Clone)]
struct NodeShutdown;

#[derive(Clone)]
struct NodeShutdownHandle;

impl NodeShutdownHandle {
    fn cancel(&self) {}
}

fn node_shutdown_channel() -> (NodeShutdownHandle, NodeShutdown) {
    (NodeShutdownHandle, NodeShutdown)
}

mod runtime {
    use super::*;

    pub(crate) async fn run_node_service(
        _home: &Path,
        _shutdown: NodeShutdown,
    ) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NodeNativeServiceState {
    NotInstalled,
    Stopped,
    Running,
}

#[derive(Clone, Copy)]
enum NodeNativeServiceError {
    AlreadyInstalled,
    ActionFailed,
    ManagerUnavailable,
    WindowsCredentialsRequired,
}

mod windows;
#[path = "service_runtime.rs"]
mod service_runtime;
EOF

RUSTFLAGS="-D warnings" \
  cargo check --offline --quiet --tests --manifest-path "$WORK_DIR/Cargo.toml" --target "$TARGET"
printf 'captain-node Windows service API check passed\n'
