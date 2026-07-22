# Memory family

> **Status:** audited (D.6).
> See [`README.md`](README.md) for the index and drift policy.
> Tool name list pinned in [`captain_runtime::captain_docs::MEMORY_FAMILY_TOOLS`](../../crates/captain-runtime/src/captain_docs.rs).

## Tools

### `memory_context_batch`

Grouped context retrieval. Runs several focused lookups across memory, session
recall, and optionally the structured knowledge graph in one tool call. Use it
when the user asks for a decision or comparison that needs multiple remembered
facts, past-session context, and project knowledge without burning a tool turn
per query.

The memory side is **high-confidence by default**. Captain reads the local
`memory_writes` journal first, then MemPalace/graph recall. Candidates are
parsed, filtered, and compacted before they reach the model. A candidate is kept
only when it overlaps enough with the query terms or carries a strong 0..1
similarity score. Low-confidence candidates are counted under `filtered` and
must not be treated as facts.

| Field | Required | Notes |
|---|---|---|
| `queries` | yes | One to thirty focused lookup strings. `query` is also accepted for a single lookup. |
| `include_memory` | no | Default true. |
| `include_sessions` | no | Default true. |
| `include_knowledge` | no | Default false; use for structured entity/relation checks. |
| `max_results` | no | Session result budget per query, default 5. |
| `memory_max_results` | no | High-confidence memory matches per query, default `max_results`, capped at 10. |
| `memory_min_similarity` | no | Strong MemPalace similarity threshold, default 0.75. |
| `strict_memory_filter` | no | Default true. Disable only for explicit forensic inspection. |
| `stop_on_error` | no | Default false. |

Captain has one primary long-term memory write path, indexed by MemPalace:

- **Long-term declarative facts** — `memory_save` first commits a `(subject, predicate, object, category)` triple to the local `memory_writes` continuity journal, then synchronizes the MemPalace semantic index. Local recall remains available while that index is down. The write broadcasts a 🧠 event and survives sessions, crashes, and restarts. This is the **default** memory tool whenever the user states a durable fact.
- **Legacy key-value store** — `memory_store` is kept for old skills and temporary coordination state. It is not the default path for durable user/project knowledge.

Prompt continuity contract: Captain injects a compact persistent memory capsule
and automatic semantic recall on normal LLM turns, including streaming turns.
When the user references earlier exchanges (`tu te souviens`, `on avait dit`,
`l'autre fois`, a named old topic, etc.), Captain should retrieve before
answering. Use `memory_context_batch` for multi-fact or past-session questions,
`memory_recall` for one focused durable fact, and `session_recall` for a
specific previous conversation.

Explicit write opt-out contract: when the current user turn clearly says not to
remember, save, learn, retain, or add long-term memory from that message,
Captain must not derive semantic graph facts, MemPalace mirrors, reflections,
conversation-learning signals, or workflow learning from the turn. This
instruction takes precedence over remember-like wording in the same message.
The normal conversation transcript and mandatory operational/audit records are
still retained; the opt-out is not an instruction to hide tool execution or
erase the session. It is per-turn and does not retract an older fact. Use
`memory_forget` when the user asks to remove or correct knowledge already
stored.

`0.1.0-alpha.6`, `0.1.0-alpha.7`, `0.1.0-alpha.8`, and `0.1.0-alpha.9` share one known
limitation: the post-turn paths above
honor this opt-out, but the core agent-loop finalizer still stores one local episodic interaction
fragment. This is distinct from the expected transcript
and audit retention. Do not promise complete per-turn semantic suppression on
any of these versions; a later immutable release must close that finalizer path.

### `memory_save`

Captain-native declarative learning. Use **spontaneously** when the user states a preference, a personal fact, a project decision, or asks you to remember something.

| Field | Required | Notes |
|---|---|---|
| `subject` | yes | `user`, `project:foo`, `agent`, `host:server`, … |
| `predicate` | yes | `prefers`, `has_dog`, `lives_in`, `runs_at`, … |
| `object` | yes | The value. PII filter rejects credentials. |
| `category` | yes | One of `info`, `skill`, `error_success`, `solution`, `other` (case-sensitive). |

Side effect: a `MemoryStored` event is broadcast on the kernel bus → 🧠 notice surfaces in the active chat (Telegram, TUI, API/SSE, …) so the user sees the fact landed. Captain does not need to guess the `channel` field during a live turn: the runtime propagates the current origin channel to `memory_save` automatically.
Daemon-backed Web/API turns and direct TUI/CLI streaming turns both provide a
live kernel handle to the tool. Captain must confirm persistence only after a
successful receipt; on any tool error, it must state that nothing was stored.
The tool receipt distinguishes `index=sync` from
`local=durable · index=pending/retry-auto` or
`index=degraded/retry-auto`; Captain must not claim remote synchronization
when only the durable local commit has completed.

Correction ordering is strict and dependency-aware: retrieve the exact old
triple, call `memory_forget` and wait for its result, then call `memory_save`
with the replacement. Never add the replacement first. This keeps local active
context and the remote semantic index coherent throughout the correction.

### `memory_recall`

Look up previously stored context. With the MemPalace backend, `key` is treated as the lookup/query string and Captain combines MemPalace results with local graph memory when available.

| Field | Required | Notes |
|---|---|---|
| `key` | yes | Focused lookup string. It can be an exact key for legacy KV data or a semantic query for MemPalace-backed memory. |

Returns a "No value found" style response when nothing matches — that's not an error, it means Captain has no stored context for that lookup.

### `memory_store`

Legacy KV write. Treat as scratch/compatibility — entries may survive across sessions, but they do not carry the explicit `subject/predicate/object/category` contract that MemPalace uses for high-quality long-term memory. Prefer **`memory_save`** for every durable fact; use `memory_store` only for agent bookkeeping, temporary workflow pointers, or old skills that still require a flat key/value API.

| Field | Required | Notes |
|---|---|---|
| `key` | yes | Stable string identifier. |
| `value` | yes | Any JSON-serialisable shape. |

### `memory_forget`

Durable retraction tool — makes facts written by `memory_save` or reflection inactive while preserving their audit rows. It atomically queues a MemPalace `kg_invalidate` operation for every matched fact. Use **spontaneously** when the user says "tu te trompes", "oublie ça", "ce n'est plus vrai", or "corrige ce que tu sais sur X".

| Field | Required | Notes |
|---|---|---|
| `subject` | at least one | SQL `LIKE` pattern (`user`, `project:%`, …). |
| `predicate` | at least one | SQL `LIKE` pattern (`has_dog`, `works_%`). |
| `object` | at least one | SQL `LIKE` pattern (`%ancienne_valeur%`, `remote%`). |

The three filters are combined in **AND**. Prefer the exact old subject,
predicate, and object for corrections; `%` is available for deliberate broad
retractions. With **no filter at all**, the call fails before changing anything
(anti-wipe guard).

`memory_forget` also records a precise active retraction guard. Archived history is not rewritten: checkpoints, journals, historical mirrors, and the original `memory_writes` row remain auditable. Active local recall excludes the row immediately. Any stale prompt or MemPalace diary hit matching the old fact is filtered, and mutable active summaries such as `canonical_sessions.compacted_summary` are sanitized when the kernel supports it. A fully exact legacy triple absent from the local journal still receives a durable MemPalace invalidation.
On restart, Captain rebuilds missing active guards from retracted journal rows,
closing the crash window between the atomic journal mutation and its auxiliary
KV snapshot.

Returns `retracted`, `invalidations_queued`, `remote_synced`, `remote_pending`,
and `remote_failed`, plus `active_context_suppressed` and
`active_context_sanitized`. `remote_pending` is not data loss: the operation is
in the local journal and the background worker will replay it.

## Sandbox

- **Managed native runtime** — MemPalace is a core dependency when
  `memory.backend = "mempalace"`. Official host installers and the published
  container provision it before first use. Every active local kernel entrypoint
  (`captain start`, direct CLI, TUI, and Captain's MCP server) then runs the same
  preflight before boot; users do not install a Python package manually. Captain
  pins uv 0.11.28, CPython 3.13.14,
  MemPalace 3.5.0, and every Python artifact through the embedded frozen
  `uv.lock`. The managed runtime lives under
  `$CAPTAIN_HOME/native/mempalace`; memory data remains separate under
  `$CAPTAIN_HOME/data/mempalace` (or an existing `~/.mempalace`, preserved in
  place on default-home upgrades). No system Python or provider API key is
  required.
- **Fail-closed readiness** — `captain memory status --json` exposes runtime,
  palace, platform, private-permission, generation, and pin state. `captain
  memory doctor` verifies the exact executable, opens the palace, and performs
  a real semantic search; it exits non-zero when any layer is degraded.
  `captain memory install --force` repairs it. Every active local kernel
  entrypoint performs the same live readiness check and automatic repair before
  boot, including a statically present runtime whose executable, model, palace,
  or permissions are broken. If repair fails, that surface does not claim production readiness.
  Setting
  `CAPTAIN_MEMPALACE_INSTALL=0` is an explicit degraded-mode opt-out, not a
  successful production install.
- **Crash-safe repair** — installs hold an interprocess lock and build a new,
  immutable runtime generation. Captain validates the exact Python and
  MemPalace versions, palace storage, and a real semantic search before an
  atomic metadata switch. Failed or interrupted generations never replace the
  active one. Runtime and memory roots are owner-only on Unix; managed metadata
  is mode `0600`. Captain retains only the active generation and one rollback
  generation, so repeated repairs cannot grow disk use without bound. Managed
  command timeouts terminate the complete process tree. Status distinguishes
  compatible, stale, and genuinely incomplete generations.
- **Version-coherent bridge** — the core MemPalace MCP bridge is launched
  through the exact Captain executable that booted the kernel, never an
  unrelated `captain` found earlier on `PATH`. An operator-defined MCP server
  named `mempalace` remains an explicit override.
- **Durable continuity and semantic index** — `memory_save`, `memory_recall`,
  and `memory_forget` are the long-term memory contract. The local
  `memory_writes` journal under Captain's data directory is the durable record
  of accepted add/invalidate operations. MemPalace is the semantic index
  derived from that journal. Pending and degraded `error` rows remain active
  for local recall and retry with persisted exponential backoff; they are never
  age-deleted automatically.
- **Outage isolation** — a resync tick processes at most 250 due operations and
  stops after the first backend failure, so one outage cannot consume the
  retry budget of the whole queue. `error` means observable degradation after
  repeated failures, not terminal loss. `captain doctor` and
  `/api/learning/metrics` expose backlog age, next retry, maximum attempts, and
  the bounded last error.
- **Active memory vs archive** — active prompt markdown such as `SOUL.md`, `AGENTS.md`, `STYLE.md` and
  global `~/.captain/USER.md` is context, not independent memory truth.
  Checkpoints and journals are archival history; do not delete them just to
  forget a fact. Use `memory_forget`, which retracts the active local fact,
  journals the MemPalace invalidation, and prevents stale archived lines from
  returning through active context or `memory_recall`.
- **MemPalace exposure** — the bundled MemPalace integration is local stdio and needs no API key. If a user exposes MemPalace as a remote MCP/SSE endpoint, require a bearer token on the server and set the MCP `auth_token_env` field to a vault-backed env var; never publish an unauthenticated memory endpoint.
- **Workspace memory markdown is retired** — workspace `MEMORY.md`,
  `USER.md`, `BOOTSTRAP.md` and `PLAYBOOK.md` are legacy migration artifacts,
  not active prompt context and not editable product surfaces. The user profile
  lives in global `~/.captain/USER.md`; durable learned facts live in MemPalace
  through `memory_save`, `memory_recall` and `memory_forget`.
- **PII filter** — `memory_save` and the async reflection pipeline reject triples that match the credential / phone-number / credit-card / personal-data regex bundle. Store secrets with `secret_write`, never with memory tools.
- **Auto-learning de-duplication** — reflection-generated facts are compared against recent durable `memory_writes` rows with normalized subject/predicate matching plus object similarity and salient-token overlap. Similar wording such as "validation via Telegram buttons" vs "learning approvals go to Telegram with interactive buttons" must collapse into one durable fact instead of creating duplicates.
- **Legacy memory guard** — `memory_store` also refuses obvious raw credential literals. It is scratch/compat state, not a vault.
- **Visible learning contract** — every accepted or queued learning must create chat feedback. Auto commits surface as `🧠 mémorisé`; approval-mode candidates surface as `💭 apprentissage à valider` with the review id. On Telegram, the inline buttons resolve the learning queue directly (`/learn_approve`, `/learn_reject`), not the generic tool approval queue. The user must never have to inspect logs to know what Captain tried to learn.
- **Visible adaptation contract** — when a learning changes future behaviour, Captain must state the behavioural delta in the current chat: what changed, why it changed, and how it will act differently next time. If the preference is ambiguous, ask one short clarification before saving instead of freezing a guess.
- **Cross-agent visibility** — every agent shares the same MemPalace by default. To scope a fact to one agent, encode the agent name in the `subject` (`agent:coder:prefers …`).
- **Silent personalization** — recalled memories are private context. Use them to adapt tone, defaults and decisions; do not list personal memories to demonstrate recall unless the user explicitly asks what Captain knows or asks about that exact subject.

## Limites

- `memory_save` rejects unknown categories with a clear error — keep to `info`, `skill`, `error_success`, `solution`, `other` (case-sensitive). The category drives downstream filters, so adding a new one needs a code change, not just a free-text label.
- `memory_recall` is ranked across the configured backend where possible; refine the key/query rather than expecting a full dump.
- `memory_context_batch` is stricter than `memory_recall`: it is optimized for
  answer context, not exhaustive search. It should still surface exact durable
  facts recently written through `memory_save` because it reads `memory_writes`
  directly. If it returns `match_count:0` with filtered candidates, do not infer
  facts from those candidates; run a narrower `memory_recall` or set
  `strict_memory_filter=false` only when explicitly auditing noisy memory.
- `memory_recall` results are not a permission to disclose. Quote the minimum relevant detail, and keep unrelated personal facts out of the answer.
- `memory_forget` requires at least one filter — calling it with `{}` returns an error before any mutation. Use SQL `LIKE` wildcard `%` deliberately; `%` alone matches everything in that field. Exact triples are the production path for corrections.
- `memory_store` values larger than ~512 KB are stored anyway but slow `memory_recall` proportionally. For large blobs use `file_write` and store the path.
- The 🧠 channel notice from `memory_save` is automatic for live routed turns. Headless/programmatic tool calls without an origin channel still broadcast on the kernel bus, but external adapters cannot infer a private chat target.
- Retraction preserves the original audit row but removes it from active recall. Re-assert a fact only when the user states it again; do not resurrect it from archival history.

## Exemples

### Golden path — save a preference, recall it later

```
1. memory_save({
     "subject": "user",
     "predicate": "prefers",
     "object": "vert sapin",
     "category": "info"
   })
   → "🧠 mémorisé : user prefers vert sapin"
2. (later session)
   memory_recall({"key": "user prefers colour"})
   → matching MemPalace/local memory context
```

### Golden path — correct a wrong fact

```
1. memory_forget({"subject":"user","predicate":"prefers_response_style","object":"long answers in English"})
   → {"status":"ok","retracted":1,"invalidations_queued":1,"remote_synced":1,"remote_pending":0,"active_context_suppressed":true}
2. memory_save({"subject":"user","predicate":"prefers_response_style","object":"short answers in French","category":"info"})
   → "🧠 mémorisé · user/prefers_response_style (info)"
```

### Error case — empty filter set is refused

```
memory_forget({})
→ Err("memory_forget refuses to retract with no filter — provide at least subject, predicate or object").
```

The anti-wipe guard is the contract: a hallucinated empty call cannot retract the journal or the MemPalace index.
