# SSH family

> **Status:** audited (D.5).
> See [`README.md`](README.md) for the index and drift policy.
> Tool name list pinned in [`captain_runtime::captain_docs::SSH_FAMILY_TOOLS`](../../crates/captain-runtime/src/captain_docs.rs).

## Tools

### `ssh_health_check`

Grouped remote health check. Use this before composing a large `ssh_exec`
script for requests like "check my VPS", "are the logs OK?", or "is service X
healthy?". Captain generates a bounded command that reports host, uptime, disk,
memory, load/CPU, failed systemd units, optional Docker containers, listening
ports, recent critical logs, and optional service status/logs.

| Field | Required | Notes |
|---|---|---|
| `key_name` | yes | Vault alias. |
| `service` | no | Optional systemd service name. Strictly validated before shell use. |
| `include_docker` | no | Default true. |
| `include_ports` | no | Default true. |
| `include_logs` | no | Default true. |
| `log_lines` | no | Default 80, capped at 200. |
| `timeout_secs` | no | Default 60, capped at 180. |

All three tools embed `russh` (and `russh-sftp` for the file ops) — there is **no shell-out to the system `ssh` binary**, no `~/.ssh/config` parsing, no `ssh-agent` reuse. Keys are stored encrypted in the Captain vault and addressed by **alias**, never by path.

### `ssh_exec`

Run a remote shell command. The default for "check the VPS", "tail the prod log", "is service X up".

| Field | Required | Notes |
|---|---|---|
| `key_name` | yes | Alias in the vault (`prod-server`, `desktop`, …). |
| `command` | yes | Remote shell line. |
| `timeout_secs` | no | Explicit value is a renewable review window for the remote command, max 7200; omitted value uses the 60s hard guard. |

Returns `{stdout, stderr, exit_code}` formatted. Critical patterns (`rm -rf /`, `mkfs`, `:(){:|:&};:`, …) are matched **before** sending — Captain's policy adds a layer that does not depend on the remote sshd config.

This tool is **proactive on purpose**: when the user mentions an alias (`prod-server`, `mon serveur`, `la machine de prod`), Captain reaches for `ssh_exec` directly instead of asking for the IP/host. The guidance is in the description so the LLM doesn't fall back to `ask_user`.

### `ssh_upload`

Push a local file to a remote path via SFTP.

| Field | Required | Notes |
|---|---|---|
| `key_name` | yes | Vault alias. |
| `local_path` | yes | Workspace-relative or absolute. |
| `remote_path` | yes | Absolute remote path. |
| `timeout_secs` | no | Default 120. |

Reads/writes happen entirely in memory — **no streaming**. Suitable for config files, scripts, snippets up to a few MB. For large transfers (gigabyte-scale logs, datasets) fall back to `shell_exec` + `rsync` over SSH.

### `ssh_download`

Pull a remote file to the local filesystem via SFTP. Parent directories of `local_path` are created automatically.

| Field | Required | Notes |
|---|---|---|
| `key_name` | yes | Vault alias. |
| `remote_path` | yes | Absolute remote path. |
| `local_path` | yes | Workspace-relative or absolute. |
| `timeout_secs` | no | Default 120. |

Same in-memory limit as `ssh_upload`.

## Sandbox

- **No raw key access** — Captain never sees `~/.ssh/id_*` directly. Those paths are in the workspace blocklist (zero access). Keys reach the SSH client only through `vault::get_decrypted_key(alias)`.
- **Vault encryption** — keys are stored in the Captain vault and decrypted on demand inside the daemon process, never written back to disk.
- **Critical-pattern filter** — `ssh_exec` reuses the same blocklist as `shell_exec` before sending the command. A remote `rm -rf /` is rejected by Captain even if the remote box would happily honour it.
- **No agent forwarding** — `ssh -A` semantics are not implemented; a compromised remote cannot pivot back through Captain's keys.
- **Host key handling** — known host fingerprints are stored alongside the key alias. A first-seen host triggers a confirmation through the same approval path as other risky tools; a key change is treated as suspicious and rejected.
- **Timeouts upstream** — SFTP tools keep hard caps at the russh socket layer. `ssh_exec` keeps the 60s hard guard when `timeout_secs` is omitted; with explicit `timeout_secs`, setup is bounded and the running remote command is monitored as a renewable review window.

## Limites

- `ssh_exec` returns combined stdout/stderr alongside the exit code. With explicit `timeout_secs`, stdout/stderr chunks and progress are also streamed while the remote command is alive; use `process_start("ssh", ["-tt", …])` only for interactive sessions.
- SFTP tools load the entire file into memory before transfer. Files larger than ~50 MB will hit the daemon's heap budget; use `shell_exec` + `rsync -e ssh` instead.
- `ssh_upload` does not resume on failure. A timeout mid-transfer leaves the partial remote file behind — the next call should overwrite or remove it.
- `ssh_download` creates missing parent directories under `local_path`. It does **not** create them on the remote side for `ssh_upload`; pre-create remote dirs with `ssh_exec("mkdir -p …")` first.
- `key_name` is resolved in this order: exact vault alias, one unambiguous shorthand (`prod` → `prod-server` if unique), configured default key for generic requests (`server`, `remote`, `vps`), only stored key for generic requests. Ambiguous shorthands fail closed and ask for an exact alias.
- `ssh_exec` does not fall back to ssh-agent or `~/.ssh/config`. All key material must be in the Captain vault.
- Host key changes fail closed. There is no `StrictHostKeyChecking=no` switch — rotating a server's host key requires re-registering the alias.
- env_clear (B.1) does not apply on the remote side: the remote shell inherits its own login env. To strip secrets from a remote command, prefix with `env -i` inside the `command` string.

## Exemples

### Fresh remote task — discover, then execute

For a new user request such as "check my server", start with the capability
router, then use native SSH:

```
1. capability_search({"query":"check remote server health via SSH"})
2. ssh_exec({"key_name":"server", "command":"uptime; df -h /; free -h"})
```

Do not inspect Captain's SSH vault with `shell_exec`. The SSH tools already
resolve exact aliases, safe natural-language shorthand, configured defaults,
and the only registered alias when that is unambiguous.

### Golden path — check then patch

```
1. ssh_exec({"key_name": "prod-server", "command": "uptime; df -h /"})
   → {"stdout": " 14:42:01 up 47 days...\n/dev/...  87%", "exit_code": 0}
2. ssh_upload({
     "key_name": "prod-server",
     "local_path": "scripts/cleanup.sh",
     "remote_path": "/tmp/cleanup.sh"
   })
3. ssh_exec({"key_name": "prod-server", "command": "bash /tmp/cleanup.sh"})
```

### Error case — alias not in the vault

```
ssh_exec({"key_name": "staging-server", "command": "echo hi"})
→ Err("No SSH key named 'staging-server'. Known aliases: prod-server. Retry with an exact alias, or call captain_docs({\"family\":\"ssh\",\"query\":\"alias not found recovery\"}) before asking the user. Do not diagnose Captain's SSH vault through shell_exec.").
```

The error is verbose by design: it tells Captain exactly what to do next without falling back to `ask_user`.

## Recovery protocol for agents

When an SSH tool fails, do this before asking the user:

1. Read the tool error literally. If it names known aliases or says the request is ambiguous, retry once with the exact alias.
2. Call `captain_docs({"family":"ssh","query":"alias resolution recovery"})` if the next step is unclear.
3. Keep using native SSH tools (`ssh_exec`, `ssh_upload`, `ssh_download`). Do not run `captain ssh list` through `shell_exec`: subprocesses have a reduced environment and can produce false vault/keyring failures.
4. Ask the user only when no alias/default/key can be inferred from session memory, memory, config, knowledge, docs, or the tool error itself.
