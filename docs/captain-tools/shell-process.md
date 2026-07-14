# Shell + Process family

> **Status:** audited (D.2).
> See [`README.md`](README.md) for the index and drift policy.
> Tool name list pinned in [`captain_runtime::captain_docs::SHELL_PROCESS_FAMILY_TOOLS`](../../crates/captain-runtime/src/captain_docs.rs).

## Tools

### One-shot execution

#### `shell_exec`

Run a shell command, capture combined stdout+stderr, return the exit code.

| Field | Required | Notes |
|---|---|---|
| `command` | yes | Full shell line (`cargo build --release`, `ls -la /tmp`). |
| `timeout_seconds` | no | Explicit value is a bounded review window with a hard cap; omitted value keeps the short default guard. |

Use **`file_read`** / **`file_write`** for plain file ops (safer, sandboxed). Reach for `shell_exec` for anything that needs a real shell. Critical patterns (`rm -rf /`, `mkfs`, …) are blocked at the policy layer before the spawn — see Sandbox below.

Do not use `shell_exec` to start a server, watcher, REPL, or any intentional
background process. `nohup`, `disown`, unquoted `&`, and nested shell
backgrounds are refused because they hide process lifecycle from Captain. Use
`process_start` instead so the agent can inspect, poll, write to, list, and kill
the process later.

#### `execute_code`

Inline Python / Node / Bash snippet without creating a file.

| Field | Required | Notes |
|---|---|---|
| `code` | yes | Source string. |
| `language` | no | `python` (default), `node`, `bash`. |
| `pip_install` | no | Allowlisted packages installed before run (Python only). |
| `timeout_secs` | no | Default 60, max 300. |

Use this for quick API probes, on-the-fly data munging, or prototyping a script. For persistent code, **`file_write` then `shell_exec`** is the right path. The subprocess inherits a stripped env (B.1: PATH, HOME, TMPDIR/TMP/TEMP, LANG/LC_ALL, TERM only), so snippets that need API keys must run through a native integration or a per-skill `env_inject` path.

Never embed the raw API key in the snippet or command. If an API call requires a credential, store it with `secret_write`, confirm with masked `secret_read`, then use a native integration or a skill with `[requirements.env_inject]`. `execute_code`, `shell_exec`, `docker_exec`, `process_start` and `process_write` refuse obvious raw secret literals and return a recovery hint instead of running.

### Native multi-tool parallelism

When one model turn emits several tool calls, Captain may overlap only calls
from an explicit read-only allowlist and only when their inputs are independent.
The classifier fails closed: unknown tools, MCP tools, skills, custom tools,
side-effecting operations, and overlapping file paths remain sequential. PRE
checks and POST bookkeeping are always ordered even for a parallel-safe EXEC
group, and results are returned in the model's original call order.

Do not infer independence merely because calls appeared in the same model
response. If one call needs another call's output, keep them sequential. For
long-running dependency graphs, use detached runs and declare `depends_on`.

### Detached tool runs

Use detached tool runs when an eligible diagnostic, build, SSH check, or package
command may run longer than one agent turn, or when several independent checks
can run in parallel without depending on each other's results.

Eligible first-level target tools:

- `shell_exec`
- `ssh_exec`
- `ssh_health_check`
- `execute_code`
- `cargo`
- `npm`
- `pip`

#### `tool_run_start`

Start one eligible tool in the background and return a `run_id` immediately.

| Field | Required | Notes |
|---|---|---|
| `tool` | yes | Target tool name, for example `shell_exec` or `ssh_exec`. |
| `input` | yes | JSON input passed to the target tool. The target tool's normal validation still applies. |
| `depends_on` | no | Array of `run_id`s that must already be `completed` successfully before this run may start. |

Parallelism rule: launch multiple `tool_run_start` calls only when the probes
are independent. If one check needs another check's output, either wait for
`tool_run_result` first or pass the prior `run_id` in `depends_on`; Captain will
refuse to start when dependencies are still running or failed.

#### `tool_run_status`

Read the current state of a run: `running`, `completed`, `failed`, or
`cancelled`, with duration and a bounded output preview.

| Field | Required | Notes |
|---|---|---|
| `run_id` | yes | The id returned by `tool_run_start`. |

#### `tool_run_result`

Read the bounded result of a completed/failed/cancelled/interrupted run, or the
current state if it is still running.

| Field | Required | Notes |
|---|---|---|
| `run_id` | yes | The id returned by `tool_run_start`. |

#### `tool_run_cancel`

Cancel a running detached tool.

| Field | Required | Notes |
|---|---|---|
| `run_id` | yes | The id returned by `tool_run_start`. |

#### `tool_run_list`

List recent detached tool runs. The newest 200 terminal runs survive restarts;
every still-running row is retained and becomes `interrupted` after a restart.
Use this history before starting duplicate diagnostics.

| Field | Required | Notes |
|---|---|---|
| `status` | no | Optional filter: `running`, `completed`, `failed`, `cancelled`, or `interrupted` after a Captain restart. |
| `limit` | no | Maximum runs to return. |

#### `docker_exec`

Run a command inside a sandboxed Docker container with network isolation, capability drops, and resource limits.

| Field | Required | Notes |
|---|---|---|
| `command` | yes | Executed inside the configured image. |
| `timeout_secs` | no | Explicit value is a renewable review window, max 7200; omitted value uses the configured Docker hard guard. |

Reach for this when you have to run untrusted code (a snippet from a web search, a build step from an unfamiliar repo). For your own commands, **`shell_exec` is faster** — `docker_exec` adds 200–500 ms of container startup. Requires `docker.enabled = true` in `config.toml`.

### Persistent processes

Long-running children survive across tool calls; up to 5 per agent. Inputs and outputs go through `process_*` siblings.

#### `process_start`

Spawn a long-lived process and return a `process_id`.

| Field | Required | Notes |
|---|---|---|
| `command` | yes | Executable (`python`, `node`, `npm`, …). |
| `args` | no | Arg vector (`["-i"]` for an interactive REPL). |
| `cwd` | no | Working directory for the process, e.g. the project folder containing `app.py` or `package.json`. |

The same env_clear whitelist as `execute_code` applies (B.2): the spawned process never sees Captain's secrets. Output is buffered in 1000-line ring buffers per stream so old output is dropped, not the new one. Live process metadata is checkpointed under `data/process_registry.json`; after a daemon restart, still-running host PIDs are listed as recovered/detached with operator actions so operators can stop them explicitly.

Use this for local apps, dev servers, REPLs and watchers that should survive the
current agent turn. Provide `cwd` when the command must run from a project
directory, then use `process_poll`, `process_list`, `process_write` and
`process_kill` as the supervision loop.

#### `process_poll`

Drain accumulated stdout/stderr without blocking. Returns whatever is buffered since the previous poll, or empty strings.

| Field | Required | Notes |
|---|---|---|
| `process_id` | yes | The id from `process_start`. |

Loop with a small sleep when watching progress; do **not** poll in a tight loop — the underlying buffer mutex is contended.

#### `process_write`

Send data to the child's stdin. A trailing newline is appended automatically when missing.

| Field | Required | Notes |
|---|---|---|
| `process_id` | yes | The id from `process_start`. |
| `data` | yes | Anything the child expects (REPL command, JSON line, …). |

Combine with `process_poll` to read the response back.

#### `process_kill`

Terminate a running process and free its file descriptors / ports / memory. Irreversible.

| Field | Required | Notes |
|---|---|---|
| `process_id` | yes | The id from `process_start`. |

#### `process_list`

Inspect the agent's currently-running processes — their ids, commands, uptime, idle time since last observed activity, whether they're still alive, and whether Captain still has attached stdin/stdout handles.

No parameters.

### Structured language wrappers

Each wrapper accepts a whitelisted `subcommand`, an `args` array, and optional `timeout_seconds`. Arguments containing shell metacharacters (`;`, `|`, `` ` ``, …) are rejected — the wrappers exist precisely to short-circuit shell injection on routine invocations.

| Field | Required | Notes |
|---|---|---|
| `subcommand` | yes | One of the wrapper-specific allowed subcommands below. |
| `args` | no | Plain arguments only; shell metacharacters are rejected. |
| `timeout_seconds` | no | Explicit value is forwarded to `shell_exec` as a bounded review window for long build/test/install/download work. |

#### `cargo`

Sub-commands: `build, test, run, check, clippy, fmt, doc, tree, update, install, version, search`. For exotic invocations fall back to `shell_exec`.

#### `npm`

Sub-commands: `install, ci, run, test, build, list, outdated, audit, version, view`. Use `shell_exec` for `publish` or other mutating commands not in the list.

#### `pip`

Sub-commands: `install, list, freeze, show, check, search, download`. Package security still rides on the external `pip-allowlist` configured elsewhere — this wrapper only blocks shell injection.

## Sandbox

- **env_clear (B.1, B.2)** — every spawn (`shell_exec`, `execute_code`, `process_start`, the wrappers, the per-skill runtimes) goes through `env_sandbox::apply_minimal_env`, which drops the parent env and re-attaches only `PATH`, `HOME`, `LANG`, `USER`. API keys held by the daemon never reach the child.
- **Per-skill secrets (B.3)** — when the spawn runs a skill, the manifest's `[requirements.env_inject]` map decides which entries from `~/.captain/secrets.env` cross over (and under which target name). Other skills' secrets stay invisible.
- **No raw secrets.env sourcing** — `~/.captain/secrets.env` is not a shell profile. Do not run `source ~/.captain/secrets.env`, `. ~/.captain/secrets.env`, or `set -a` around it: some entries can be logical Captain identifiers rather than shell-safe variable names. Use `secret_read`, a native integration, or skill `env_inject`.
- **Critical patterns** — destructive commands (`rm -rf /`, `mkfs`, `:(){:|:&};:`, `dd of=/dev/sda`, …) are matched and rejected before spawn (`critical_patterns`). The block also covers `shell_exec` snippets that contain those patterns inside `eval` / `bash -c` payloads.
- **Docker isolation** — `docker_exec` adds another layer: read-only rootfs, dropped capabilities, no `--privileged`, optional network namespace.
- **Per-agent quota** — `process_*` is capped at 5 concurrent processes per agent.
- **CWD** — one-shot spawns run with `current_dir` set to the agent's workspace root (or the skill dir for `skill_execute`). `process_start` may override this with its `cwd` field for supervised project processes. `..` escapes are rejected upstream (`validate_path`) where a workspace-relative path is expected.

## Limites

- `shell_exec` default timeout is 30 s; set `timeout_seconds` explicitly for planned long one-shot work. Explicit timeouts are reviewed with progress and then hard-capped; if the cap is reached, cleanup is scheduled without trapping the agent turn. Persistent servers/watchers belong to `process_start`, and detached shell forms (`nohup`, `disown`, unquoted `&`, nested `bash/sh/zsh -c "... &"`) are rejected.
- `execute_code` caps explicit review windows at 300 s. Without `timeout_secs`, the default 60 s remains a hard guard; with `timeout_secs`, a live process emits progress and keeps running. Python `pip_install` is restricted to an allowlist (`requests, httpx, beautifulsoup4, lxml, pandas, numpy, pyyaml, python-dateutil, pyobjc-framework-Quartz, pillow`). For a package outside the list use a skill with declared requirements rather than expanding the allowlist ad-hoc.
- `docker_exec` uses `docker.timeout_secs` as a hard guard when `timeout_secs` is omitted. With explicit `timeout_secs`, the value is a review window: a live container command emits progress and is not killed at the first deadline. Use `process_start` for servers/watchers that should run indefinitely.
- `process_*` buffers each stream at 1000 lines; older lines are dropped silently. Capture them with `process_poll` early if the child is verbose. Long-runners track last stdout/stderr or stdin activity; cleanup reaps old exited handles but does not kill a live process only because it is old.
- `process_kill` does not flush the child's stdout — drain with `process_poll` first if the last lines matter.
- `cargo` / `npm` / `pip` block any argument containing shell metacharacters; that catches `--feature foo;rm -rf /` but also rejects intentional shell substitutions — fall back to `shell_exec` for those.
- `cargo` / `npm` / `pip` keep the short shell guard when `timeout_seconds` is omitted. For long `cargo test`, `npm install`, `npm run build`, `pip install`, or `pip download`, set `timeout_seconds` explicitly so active work is reviewed instead of killed at the first deadline.
- `docker_exec` requires `docker.enabled = true` in `config.toml`. Without it the call returns `"docker disabled"` and never spawns anything.
- env_clear means a child running `python -c "import os; print(os.environ)"` sees only the minimal safe env (`PATH`, `HOME`, temp-dir vars, locale vars, `TERM`). Snippets that assume `OPENAI_API_KEY` is in env will fail unless a first-class integration or skill manifest injects it. Do not pass raw secrets as CLI args; those are command literals and are blocked.

## Exemples

### Golden path — one-shot shell + structured wrapper

```
shell_exec({"command": "git status --porcelain", "timeout_seconds": 5})
→ {"stdout": " M Cargo.toml\n", "stderr": "", "exit_code": 0}

cargo({"subcommand": "test", "args": ["-p", "captain-runtime", "--lib"], "timeout_seconds": 900})
→ {"exit_code": 0, "stdout": "test result: ok. 1345 passed; …"}
```

### Golden path — persistent REPL session

```
1. process_start({"command": "python", "args": ["-iu"]})
   → {"process_id": "proc_42"}
2. process_write({"process_id": "proc_42", "data": "import json; print(json.dumps({\"a\":1}))"})
3. process_poll({"process_id": "proc_42"})
   → {"stdout": ["{\"a\": 1}"], "stderr": []}
4. process_kill({"process_id": "proc_42"})
   → {"status": "killed"}
```

### Error case — env_clear preventing a secret leak

```
execute_code({
  "language": "python",
  "code": "import os; print(os.environ.get('OPENAI_API_KEY', 'absent'))"
})
→ {"stdout": "absent\n", "stderr": "", "exit_code": 0}
```

The literal `absent` is the contract: the daemon's API key is held in its own env but never propagates to the child.
