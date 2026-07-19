#!/usr/bin/env python3
"""Drive the real Ratatui surface against an isolated certification daemon."""

import argparse
import errno
import fcntl
import json
import os
import pty
import re
import select
import signal
import struct
import termios
import time
import urllib.request
from pathlib import Path


def parse_args():
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin", required=True)
    parser.add_argument("--config", required=True)
    parser.add_argument("--base", required=True)
    parser.add_argument("--artifacts", required=True)
    parser.add_argument("--timeout", type=float, default=60.0)
    return parser.parse_args()


def fetch_runs(base):
    request = urllib.request.Request(
        f"{base}/api/capabilities/native/runs?limit=500",
        headers={"X-API-Key": os.environ["CAPSPEC_CERT_API_KEY"]},
    )
    with urllib.request.urlopen(request, timeout=3) as response:
        return json.load(response).get("runs", [])


def read_available(fd, output):
    while True:
        ready, _, _ = select.select([fd], [], [], 0)
        if not ready:
            return
        try:
            chunk = os.read(fd, 16384)
        except OSError as error:
            if error.errno in (errno.EAGAIN, errno.EWOULDBLOCK, errno.EIO):
                return
            raise
        if not chunk:
            return
        output.extend(chunk)


def wait_for_any_fragment(fd, output, fragments, timeout, start=0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        read_available(fd, output)
        window = output[start:]
        for fragment in fragments:
            if fragment in window:
                return fragment
        time.sleep(0.1)
    return None


def wait_for_new_run(base, existing_ids, fd, output, deadline):
    while time.monotonic() < deadline:
        read_available(fd, output)
        try:
            runs = fetch_runs(base)
        except Exception:
            time.sleep(0.2)
            continue
        matching = [
            run
            for run in runs
            if run.get("run_id") not in existing_ids
            and run.get("capability_name") == "cert-parallel"
        ]
        if matching and matching[0].get("status") in {"succeeded", "failed"}:
            return matching[0]
        time.sleep(0.2)
    raise RuntimeError("TUI did not produce a terminal cert-parallel run")


def wait_for_child(pid, timeout):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            done, status = os.waitpid(pid, os.WNOHANG)
        except ChildProcessError:
            return 0
        if done:
            return status
        time.sleep(0.05)
    return None


def terminate_child(pid, fd):
    try:
        os.write(fd, b"\x03\x03\x03")
    except OSError:
        pass
    status = wait_for_child(pid, 2)
    if status is not None:
        return status
    try:
        os.kill(pid, signal.SIGTERM)
    except ProcessLookupError:
        return 0
    status = wait_for_child(pid, 1)
    if status is not None:
        return status
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        return 0
    return None


def main():
    args = parse_args()
    artifacts = Path(args.artifacts)
    artifacts.mkdir(parents=True, exist_ok=True)
    existing_ids = {run.get("run_id") for run in fetch_runs(args.base)}
    output = bytearray()
    failure = None
    run = None
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["TERM"] = "xterm-256color"
        os.environ.setdefault("RUST_BACKTRACE", "0")
        os.execl(args.bin, args.bin, "--config", args.config, "tui")

    try:
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 36, 128, 0, 0))
        flags = fcntl.fcntl(fd, fcntl.F_GETFL)
        fcntl.fcntl(fd, fcntl.F_SETFL, flags | os.O_NONBLOCK)
        resume_fragment = b"Reprendre la"
        chat_fragment = b"/help for commands"
        boot_screen = wait_for_any_fragment(
            fd, output, (resume_fragment, chat_fragment), 30
        )
        if boot_screen is None:
            raise RuntimeError("TUI did not reach resume or chat")
        if boot_screen == resume_fragment:
            os.write(fd, b"n")
            if wait_for_any_fragment(fd, output, (chat_fragment,), 10) is None:
                raise RuntimeError("TUI did not reach chat after declining resume")
        turn_start = len(output)
        os.write(
            fd,
            b"[CAPSPEC-CERT:tui] Inspect the real certification repository.\r",
        )
        run = wait_for_new_run(
            args.base,
            existing_ids,
            fd,
            output,
            time.monotonic() + args.timeout,
        )
        if (
            wait_for_any_fragment(
                fd, output, (b"CAPSPEC_CERT_OK",), 30, start=turn_start
            )
            is None
        ):
            raise RuntimeError("TUI model turn did not render its final response")
        completion_start = output.find(b"CAPSPEC_CERT_OK", turn_start)
        if (
            wait_for_any_fragment(
                fd, output, (b"pr\xc3\xaat",), 10, start=completion_start
            )
            is None
        ):
            raise RuntimeError("TUI did not return to idle after the model turn")
        time.sleep(1)
        navigation_start = len(output)
        os.write(fd, b"\x1b[15~")
        if (
            wait_for_any_fragment(
                fd, output, (b"Natives",), 10, start=navigation_start
            )
            is None
        ):
            raise RuntimeError("TUI native capabilities frame was not rendered")
    except Exception as error:
        failure = error
    finally:
        status = terminate_child(pid, fd)
        read_available(fd, output)
        os.close(fd)
        if status is None:
            status = wait_for_child(pid, 2)
        if status is None:
            status = -signal.SIGKILL

    raw = bytes(output)
    (artifacts / "tui-pty.log").write_bytes(raw)
    visible = re.sub(rb"\x1b(?:[@-_][0-?]*[ -/]*[@-~]|\][^\x07]*(?:\x07|\x1b\\))", b"", raw)
    text = visible.decode("utf-8", errors="replace")
    (artifacts / "tui-visible.log").write_text(text, encoding="utf-8")
    if failure is not None:
        raise RuntimeError(f"TUI certification failed: {failure}") from failure
    assert run is not None
    if run.get("status") != "succeeded":
        raise RuntimeError(f"TUI CapSpec run failed: {run}")
    if "Natives" not in text or "cert-parallel" not in text:
        raise RuntimeError("TUI native capabilities frame was not observed")
    summary = {
        "status": "passed",
        "child_status": status,
        "run_id": run.get("run_id"),
        "source_hash": run.get("source_hash"),
        "origin": run.get("origin"),
        "native_frame_observed": True,
    }
    (artifacts / "tui-summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(json.dumps(summary, sort_keys=True))


if __name__ == "__main__":
    main()
