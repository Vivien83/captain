#!/usr/bin/env bash
# Minimal reproducible smoke for the real ratatui TUI path.
#
# This smoke intentionally avoids daemon, provider and LLM calls. It only proves
# that the shipped CLI can enter the full TUI in a PTY, draw an initial frame and
# restore/exit cleanly after the standard double Ctrl+C quit path.

set -u

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKDIR="${CAPTAIN_TUI_SMOKE_WORKDIR:-$ROOT_DIR/target/tui-smoke}"
TIMEOUT="${CAPTAIN_TUI_SMOKE_TIMEOUT:-8}"
STARTUP_SECS="${CAPTAIN_TUI_SMOKE_STARTUP_SECS:-1.0}"
CAPTAIN_BIN="${CAPTAIN_BIN:-}"
HOME_DIR="$WORKDIR/home"
CONFIG="$HOME_DIR/config.toml"
PTY_LOG="$WORKDIR/tui-pty.log"

note() { printf '   %s\n' "$*"; }
pass() { printf '   ok %s\n' "$*"; }

fail() {
  printf '   FAIL %s\n' "$*" >&2
  if [ -f "$PTY_LOG" ]; then
    printf '\n--- tui pty log tail ---\n' >&2
    tail -80 "$PTY_LOG" >&2 || true
  fi
  if [ -f "$HOME_DIR/tui.log" ]; then
    printf '\n--- tui tracing log tail ---\n' >&2
    tail -80 "$HOME_DIR/tui.log" >&2 || true
  fi
  exit 1
}

usage() {
  cat <<'USAGE'
Usage: scripts/tui-smoke.sh [--bin path] [--workdir path] [--timeout seconds]

Environment:
  CAPTAIN_BIN                    Captain binary to test.
  CAPTAIN_TUI_SMOKE_WORKDIR      Artifact directory.
  CAPTAIN_TUI_SMOKE_TIMEOUT      PTY run timeout in seconds.
  CAPTAIN_TUI_SMOKE_STARTUP_SECS Seconds to wait before sending double Ctrl+C.
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --bin)
      [ $# -ge 2 ] && [ -n "${2:-}" ] || fail "missing value for --bin"
      CAPTAIN_BIN="$2"
      shift 2
      ;;
    --workdir)
      [ $# -ge 2 ] && [ -n "${2:-}" ] || fail "missing value for --workdir"
      WORKDIR="$2"
      HOME_DIR="$WORKDIR/home"
      CONFIG="$HOME_DIR/config.toml"
      PTY_LOG="$WORKDIR/tui-pty.log"
      shift 2
      ;;
    --timeout)
      [ $# -ge 2 ] && [ -n "${2:-}" ] || fail "missing value for --timeout"
      TIMEOUT="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

resolve_captain_bin() {
  if [ -n "$CAPTAIN_BIN" ]; then
    [ -x "$CAPTAIN_BIN" ] || fail "CAPTAIN_BIN is not executable: $CAPTAIN_BIN"
    return
  fi
  if [ -x "$ROOT_DIR/target/release/captain" ]; then
    CAPTAIN_BIN="$ROOT_DIR/target/release/captain"
    return
  fi
  if [ -x "$ROOT_DIR/target/debug/captain" ]; then
    CAPTAIN_BIN="$ROOT_DIR/target/debug/captain"
    return
  fi
  note "building captain CLI because no local binary exists"
  (cd "$ROOT_DIR" && cargo build -p captain-cli) || fail "cargo build -p captain-cli failed"
  CAPTAIN_BIN="$ROOT_DIR/target/debug/captain"
}

write_config() {
  mkdir -p "$HOME_DIR" "$HOME_DIR/data" "$WORKDIR"
  cat >"$CONFIG" <<EOF
home_dir = "$HOME_DIR"
data_dir = "$HOME_DIR/data"
log_level = "warn"
api_listen = "127.0.0.1:0"
network_enabled = false
api_key = ""
language = "en"

[default_model]
provider = "codex"
model = "gpt-5.5"
api_key_env = ""

[approval]
require_approval = []
EOF
}

run_pty_smoke() {
  CAPTAIN_BIN="$CAPTAIN_BIN" \
  CAPTAIN_HOME="$HOME_DIR" \
  CAPTAIN_TUI_SMOKE_CONFIG="$CONFIG" \
  CAPTAIN_TUI_SMOKE_PTY_LOG="$PTY_LOG" \
  CAPTAIN_TUI_SMOKE_TIMEOUT="$TIMEOUT" \
  CAPTAIN_TUI_SMOKE_STARTUP_SECS="$STARTUP_SECS" \
    python3 - <<'PY'
import errno
import fcntl
import os
import pty
import select
import signal
import struct
import sys
import termios
import time

captain_bin = os.environ["CAPTAIN_BIN"]
config = os.environ["CAPTAIN_TUI_SMOKE_CONFIG"]
pty_log = os.environ["CAPTAIN_TUI_SMOKE_PTY_LOG"]
timeout = float(os.environ["CAPTAIN_TUI_SMOKE_TIMEOUT"])
startup = float(os.environ["CAPTAIN_TUI_SMOKE_STARTUP_SECS"])

pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "xterm-256color"
    os.environ.setdefault("RUST_BACKTRACE", "0")
    os.execl(captain_bin, captain_bin, "--config", config, "tui")

try:
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 120, 0, 0))
except OSError:
    pass
try:
    flags = fcntl.fcntl(fd, fcntl.F_GETFL)
    fcntl.fcntl(fd, fcntl.F_SETFL, flags | os.O_NONBLOCK)
except OSError:
    pass

started = time.monotonic()
quit_step = 0
output = bytearray()
status = None

try:
    child_pgid = os.getpgid(pid)
except OSError:
    child_pgid = None

def reap_child():
    global status
    if status is not None:
        return True
    try:
        done, child_status = os.waitpid(pid, os.WNOHANG)
    except ChildProcessError:
        return True
    if done:
        status = child_status
        return True
    return False

def signal_child(sig):
    try:
        os.kill(pid, sig)
    except ProcessLookupError:
        pass
    if child_pgid is not None:
        try:
            os.killpg(child_pgid, sig)
        except (ProcessLookupError, PermissionError):
            pass

def stop_child_after_timeout():
    signal_child(signal.SIGTERM)
    deadline = time.monotonic() + 0.5
    while time.monotonic() < deadline:
        if reap_child():
            return
        time.sleep(0.05)
    signal_child(signal.SIGKILL)
    deadline = time.monotonic() + 1.0
    while time.monotonic() < deadline:
        if reap_child():
            return
        time.sleep(0.05)

while True:
    elapsed = time.monotonic() - started
    if quit_step == 0 and elapsed >= startup:
        os.write(fd, b"\x03")
        quit_step = 1
    elif quit_step == 1 and elapsed >= startup + 0.2:
        os.write(fd, b"\x03\x03")
        quit_step = 2

    ready, _, _ = select.select([fd], [], [], 0.05)
    if ready:
        try:
            data = os.read(fd, 8192)
        except OSError as exc:
            if exc.errno == errno.EIO:
                break
            if exc.errno in (errno.EAGAIN, errno.EWOULDBLOCK):
                data = b""
            else:
                raise
        if not data:
            pass
        else:
            output.extend(data)

    if reap_child():
        break

    if elapsed > timeout:
        stop_child_after_timeout()
        with open(pty_log, "wb") as f:
            f.write(output)
        print(f"TUI smoke timed out after {timeout:.1f}s", file=sys.stderr)
        sys.exit(124)

try:
    while True:
        ready, _, _ = select.select([fd], [], [], 0)
        if not ready:
            break
        try:
            data = os.read(fd, 8192)
        except OSError as exc:
            if exc.errno in (errno.EAGAIN, errno.EWOULDBLOCK, errno.EIO):
                break
            raise
        if not data:
            break
        output.extend(data)
except OSError:
    pass

with open(pty_log, "wb") as f:
    f.write(output)

if status is None:
    deadline = time.monotonic() + 1.0
    while status is None and time.monotonic() < deadline:
        reap_child()
        time.sleep(0.05)
    if status is None:
        stop_child_after_timeout()
    if status is None:
        print("TUI child did not reap after exit", file=sys.stderr)
        sys.exit(125)

if os.WIFEXITED(status):
    code = os.WEXITSTATUS(status)
    if code != 0:
        print(f"TUI exited with code {code}", file=sys.stderr)
        sys.exit(code)
elif os.WIFSIGNALED(status):
    sig = os.WTERMSIG(status)
    print(f"TUI terminated by signal {sig}", file=sys.stderr)
    sys.exit(128 + sig)
else:
    print("TUI ended with an unknown child status", file=sys.stderr)
    sys.exit(1)
PY
}

assert_clean_logs() {
  [ -s "$PTY_LOG" ] || fail "PTY log is empty; TUI did not draw"
  grep -aE 'Captain|Chat|Projects' "$PTY_LOG" >/dev/null 2>&1 ||
    fail "PTY log does not contain recognizable TUI text"
  if grep -aE 'panic|panicked|thread .* panicked|Failed to draw' "$PTY_LOG" >/dev/null 2>&1; then
    fail "PTY log contains a panic/draw failure"
  fi
  if [ -f "$HOME_DIR/tui.log" ] &&
     grep -aE 'panic|panicked|thread .* panicked|Failed to draw' "$HOME_DIR/tui.log" >/dev/null 2>&1; then
    fail "TUI tracing log contains a panic/draw failure"
  fi
}

require_cmd python3
require_cmd grep
require_cmd tail
resolve_captain_bin
write_config

note "workdir=$WORKDIR"
note "captain_bin=$CAPTAIN_BIN"

run_pty_smoke || fail "TUI PTY run failed"
assert_clean_logs
pass "full TUI entered a PTY, drew a frame and exited via double Ctrl+C"
printf '\nTUI smoke passed. Artifacts: %s\n' "$WORKDIR"
