# Session + Workspace family

> **Status:** audited (D.13).
> See [`README.md`](README.md) for the index and drift policy.
> Tool name list pinned in [`captain_runtime::captain_docs::SESSION_WORKSPACE_FAMILY_TOOLS`](../../crates/captain-runtime/src/captain_docs.rs).

## Tools

These two tools sit at the boundary between past and present: `session_recall` reads frozen summaries of previous conversations, `workspace_add` mutates the live filesystem sandbox so the current conversation can reach a new directory.

### `session_recall`

Search the auto-generated `checkpoint.md` files of past sessions. Use **spontaneously** when the user references something they said before: `on avait dit`, `l'autre fois`, `tu m'avais dit que`, `rappelle-moi ce qu'on a fait sur X`, or any conversation that is clearly post-hoc.

| Field | Required | Notes |
|---|---|---|
| `query` | yes | Multi-word, case-insensitive (terms ANDed). |
| `max_results` | no | Default 5, capped at 20. |
| `agent_filter` | no | Restrict to one agent's checkpoints (`daemon-67eae65a-…`). |

Returns a list of `{path, summary, last_modified, score}` sorted by freshness. The summaries follow a 5-section template (Sujets / Décisions / Erreurs / Réussites / Infos durables) produced by a background Haiku job that fires when a session is inactive ≥ 10 min.

For deeper exploration of a hit, open the raw JSON via `file_read` on the path returned — the `checkpoint.md` is the lossy index, the JSON is the source of truth.

### `workspace_add`

Extend Captain's authorised sandbox to a new directory. Use **spontaneously** when the user says `donne-toi accès à X`, `ouvre Y comme workspace`, `travaille sur le dossier Z`.

| Field | Required | Notes |
|---|---|---|
| `path` | yes | Absolute path to an **existing** directory. |

The path is canonicalised (symlinks resolved) and persisted into `config.toml [workspace] extra_paths`. Subsequent `file_read` / `file_write` / `glob` / `grep` calls with absolute paths under that root will succeed. Only the principal Captain agent benefits — workers spawned with `agent_spawn` still see only their own workspace.

Refused for paths that fall under the global blocklist (`~/.ssh`, `~/.gnupg`, `/etc/shadow`, …) — Captain cannot opt into reading SSH private keys.

### `session_tool_call_summary`

Return which tools were **actually executed** in the caller's own current session, sourced from the same persisted event log `captain replay` reads (`sessions_events` / `read_session_events_tail`) — not from memory or narrative recall. Use it **before** writing any self-test report, status summary, or answer that claims a capability was exercised: check that the tool in question appears in `distinct_tools_called` for this session before writing "OK" or "échoué" for it. Added after an audit found a self-authored test report re-stating a stale `speech_to_text` failure from a previous session without a matching tool call in the current one.

| Field | Required | Notes |
|---|---|---|
| `limit` | no | Max session events scanned. Default 200, capped at 2000. |

Takes no session identifier — it always resolves the caller's own current session via its agent registry entry, never another agent's. Returns `{session_id, agent_id, events_scanned, distinct_tools_called, call_counts, calls}`, where `calls` only includes `tool_execution_result` events (a tool that was *selected* but never completed does not count as tested). If called more than 3 times within 60 seconds by the same agent, the response gains a `note` field asking to call it once per test step or once before finalizing a report — added after a live run called it 35 times in under 6 minutes instead of using it as a pre-report check.

## Sandbox

- **session_recall is read-only** — it scans `~/.captain/sessions/**/checkpoint.md` (an authorised root for Captain). It never opens the raw `transcript.json` files unless the LLM follows up with an explicit `file_read`.
- **workspace_add canonicalises before persisting** — symlinks pointing into `/etc` cannot bypass the root resolution; the canonicalized real path is what gets stored.
- **session_tool_call_summary is read-only and self-scoped** — it can only read the calling agent's own session events; there is no parameter to target another agent's session.
- **Multi-root resolution** — `resolve_sandbox_path_multi` (used by every file tool) checks the agent's primary workspace first, then iterates `extra_paths`. The blocklist applies to every root, no exceptions.
- **Persistence across restarts** — `extra_paths` lives in `config.toml`, so the grant survives daemon reboots.

## Limites

- `session_recall` searches only summarised checkpoints. A session that ended too recently (< 10 min) won't have one yet — fall back to `file_list("~/.captain/sessions/")` plus `file_read` if you need the in-progress transcript.
- The 5-section template can drift: Haiku occasionally emits non-canonical headings. Always parse the response defensively rather than indexing by line number.
- `session_recall` returns matches as substrings. Long compound terms (`status-checker-v2`) match better when split (`status checker v2`).
- `workspace_add` does **not** delete a path. Removing a previously granted root requires editing `config.toml` directly.
- `workspace_add` validates that the path exists at the moment of the call. If the user later moves or deletes the directory, the entry remains in `config.toml` and surfaces as `path resolution failed` on the next file op.
- A path that resolves to one of the blocklist entries through a symlink will be rejected with the canonical-path mention so Captain can explain why.
- `session_tool_call_summary` only sees `tool_execution_result` events already persisted in the current session's event log; a tool call still in flight when the summary is requested will not appear yet.

## Per-project `.captain.toml`

The TUI also looks for a `.captain.toml` at the launch directory and walks up to `$HOME` (inclusive). When found, the file describes which agent the new session should bind to and lets the project carry its own preferences without touching the global `~/.captain/config.toml`.

```toml
# /path/to/project/.captain.toml
[captain]
# Pick exactly one of `agent` (UUID) or `agent_name` (case-insensitive).
agent_name = "captain"
# agent = "67eae65a-db95-46f6-a1a7-026e72d6a2a0"

# Optional. Surfaces project-aware memory recall later in the session.
project_slug = "captain-v3"

# Optional. ToolProfile applied when entering the chat tab
# (`minimal | coding | research | messaging | automation | full`).
tool_profile = "coding"

# Optional. Extra paths added to the kernel sandbox at bind time.
# Validated against the credential blocklist (~/.ssh, secrets.env, vault.enc, …).
extra_paths = ["/home/user/projects/example-service-shared"]
```

Lookup rules:

- The walk stops at `$HOME` (inclusive). A `.captain.toml` placed *above* the home directory is never picked up — a stray system-wide file cannot contaminate a project that did not opt in.
- A CLI override (`captain chat <agent>`) wins over `.captain.toml`.
- A malformed `.captain.toml` is logged at `warn!` and ignored — the welcome menu falls back to the manual flow rather than crashing the TUI.
- Unknown keys are tolerated so the format can grow without breaking older binaries.

## Exemples

### Golden path — recall and re-open

```
session_recall({"query": "deployment script timezone"})
→ [{
    "path": "~/.captain/sessions/.../checkpoint.md",
    "summary": "## Décisions\n- Timezone: Europe/Paris\n- ..."
   }]
file_read({"path": "~/.captain/sessions/.../transcript.json"})
→ raw transcript for the deeper context.
```

### Golden path — open a project folder

```
workspace_add({"path": "/home/user/projects/example-service"})
→ {"status":"ok","root":"/home/user/projects/example-service","persisted":true}
file_read({"path": "/home/user/projects/example-service/README.md"})
→ project README.
```

### Error case — a blocked path

```
workspace_add({"path": "/home/user/.ssh"})
→ Err("path /home/user/.ssh falls under blocklist (~/.ssh) — refusing to grant access").
```

The blocklist supersedes any grant; rotating that decision means changing the kernel, not the workspace tool.

### Verification — did I really call this tool?

```
session_tool_call_summary({})
→ {"distinct_tools_called": ["file_write", "channel_send"], "call_counts": {"file_write": 2, "channel_send": 1}, ...}
```

If a claimed capability (e.g. `speech_to_text`) is absent from `distinct_tools_called`, the report must say `NON TESTÉ CETTE SESSION`, not restate an old result as if it were fresh.
