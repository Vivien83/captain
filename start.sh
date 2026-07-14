#!/usr/bin/env bash
# Captain v3 — oneshot startup
# Usage: ./start.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
LOG_DIR="$SCRIPT_DIR/.hora/logs"
mkdir -p "$LOG_DIR"

# ── 1. Ollama embedding model ──────────────────────────────────
if command -v ollama &>/dev/null; then
  if ! ollama list 2>/dev/null | grep -q "nomic-embed-text"; then
    echo "[captain] Pulling nomic-embed-text..."
    ollama pull nomic-embed-text
  fi
fi

# ── 2. Ensure hands directory exists ───────────────────────────
mkdir -p "$HOME/.captain/hands"

# ── 3. Kill any existing captain daemon ───────────────────────
if pgrep -f "captain.*start" >/dev/null 2>&1; then
  echo "[captain] Stopping existing daemon..."
  pkill -f "captain.*start" 2>/dev/null || true
  sleep 2
fi

# ── 4. Start captain daemon ──────────────────────────────────
echo "[captain] Starting daemon..."
nohup "$SCRIPT_DIR/target/release/captain" start > "$LOG_DIR/daemon.log" 2>&1 &
DAEMON_PID=$!
echo "[captain] Daemon PID: $DAEMON_PID"

# ── 5. Wait for API health ────────────────────────────────────
echo -n "[captain] Waiting for API..."
for i in $(seq 1 15); do
  if curl -sf http://127.0.0.1:50051/api/health >/dev/null 2>&1; then
    echo " OK"
    break
  fi
  echo -n "."
  sleep 1
done

# ── 6. Start Next.js frontend ─────────────────────────────────
if [ -d "$SCRIPT_DIR/apps/web" ]; then
  # Kill any existing Next.js on port 3000
  lsof -ti:3000 2>/dev/null | xargs kill 2>/dev/null || true
  sleep 1
  echo "[captain] Starting frontend (port 3000)..."
  cd "$SCRIPT_DIR/apps/web"
  nohup npx next dev > "$LOG_DIR/frontend.log" 2>&1 &
  FRONTEND_PID=$!
  echo "[captain] Frontend PID: $FRONTEND_PID"
  cd "$SCRIPT_DIR"
fi

# ── Summary ───────────────────────────────────────────────────
echo ""
echo "  ╔══════════════════════════════════════╗"
echo "  ║       Captain v3 — Running          ║"
echo "  ╠══════════════════════════════════════╣"
echo "  ║  API:      http://127.0.0.1:50051    ║"
echo "  ║  WebChat:  http://127.0.0.1:50051/   ║"
echo "  ║  Frontend: http://localhost:3000      ║"
echo "  ║  Logs:     .hora/logs/               ║"
echo "  ╚══════════════════════════════════════╝"
