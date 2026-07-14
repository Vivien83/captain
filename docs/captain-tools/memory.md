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

Captain has one primary long-term memory write path, backed by MemPalace:

- **Long-term declarative facts** — `memory_save` writes a `(subject, predicate, object, category)` triple through the local `memory_writes` queue into MemPalace, broadcasts a 🧠 event, and survives across sessions. This is the **default** memory tool whenever the user states a durable fact.
- **Legacy key-value store** — `memory_store` is kept for old skills and temporary coordination state. It is not the default path for durable user/project knowledge.

Prompt continuity contract: Captain injects a compact persistent memory capsule
and automatic semantic recall on normal LLM turns, including streaming turns.
When the user references earlier exchanges (`tu te souviens`, `on avait dit`,
`l'autre fois`, a named old topic, etc.), Captain should retrieve before
answering. Use `memory_context_batch` for multi-fact or past-session questions,
`memory_recall` for one focused durable fact, and `session_recall` for a
specific previous conversation.

### `memory_save`

Captain-native declarative learning. Use **spontaneously** when the user states a preference, a personal fact, a project decision, or asks you to remember something.

| Field | Required | Notes |
|---|---|---|
| `subject` | yes | `user`, `project:foo`, `agent`, `host:server`, … |
| `predicate` | yes | `prefers`, `has_dog`, `lives_in`, `runs_at`, … |
| `object` | yes | The value. PII filter rejects credentials. |
| `category` | yes | One of `info`, `skill`, `error_success`, `solution`, `other` (case-sensitive). |

Side effect: a `MemoryStored` event is broadcast on the kernel bus → 🧠 notice surfaces in the active chat (Telegram, TUI, API/SSE, …) so the user sees the fact landed. Captain does not need to guess the `channel` field during a live turn: the runtime propagates the current origin channel to `memory_save` automatically.

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

Retraction tool — removes facts a previous `memory_save` (or the reflection pipeline) wrote. Use **spontaneously** when the user says "tu te trompes", "oublie ça", "ce n'est plus vrai", "corrige ce que tu sais sur X".

| Field | Required | Notes |
|---|---|---|
| `subject` | at least one | SQL `LIKE` pattern (`user`, `project:%`, …). |
| `predicate` | at least one | SQL `LIKE` pattern (`has_dog`, `works_%`). |
| `object` | at least one | SQL `LIKE` pattern (`%ancienne_valeur%`, `remote%`). |

The three filters are combined in **AND** — every supplied filter must match for a row to fall. With **no filter at all**, the call returns `0` and deletes nothing (anti-wipe guard).

`memory_forget` also records an active retraction guard. Archived history is not rewritten: checkpoints, journals and historical mirrors remain the past. But any prompt-injected context or `memory_recall` result that still contains the forgotten term is filtered before the model sees it, so an old snapshot or MemPalace diary hit cannot reintroduce the fact as current truth. Mutable active summaries, such as `canonical_sessions.compacted_summary`, are sanitized immediately when the kernel supports it.

Returns the row count deleted, `active_context_suppressed=true` when that guard was recorded, and `active_context_sanitized` with counts for mutable active summaries that were updated or cleared.

## Sandbox

- **MemPalace SSOT** — `memory_save`, `memory_recall` and `memory_forget` are the long-term memory contract. Writes first land in the local `memory_writes` table under Captain's data directory, then sync to MemPalace best-effort/asynchronously.
- **Active memory vs archive** — MemPalace/long-term graph is the canonical
  memory. Active prompt markdown such as `SOUL.md`, `AGENTS.md`, `STYLE.md` and
  global `~/.captain/USER.md` is context, not independent memory truth.
  Checkpoints and journals are archival history; do not delete them just to
  forget a fact. Use `memory_forget`, which retracts the canonical fact and
  prevents stale archived lines from being injected as active context or
  returned through `memory_recall`.
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
- `memory_forget` requires at least one filter — calling it with `{}` returns `removed: 0` instead of wiping the store. Use SQL `LIKE` wildcards (`%`) deliberately; `%` alone matches everything in that field.
- `memory_store` values larger than ~512 KB are stored anyway but slow `memory_recall` proportionally. For large blobs use `file_write` and store the path.
- The 🧠 channel notice from `memory_save` is automatic for live routed turns. Headless/programmatic tool calls without an origin channel still broadcast on the kernel bus, but external adapters cannot infer a private chat target.
- Deletion is permanent — there is no `memory_undo`. The reflection pipeline will not re-create a fact you just forgot unless the user states it again.

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

### Golden path — retract a wrong fact

```
memory_forget({"subject":"user","predicate":"prefers","object":"%ancienne_valeur%"})
→ {"status":"ok","removed":1,"active_context_suppressed":true,"active_context_sanitized":{"status":"ok","canonical_summaries_updated":1,"canonical_summaries_cleared":0}}
```

### Error case — empty filter set is refused

```
memory_forget({})
→ Err("memory_forget refuses to delete with no filter — provide at least subject, predicate or object").
```

The anti-wipe guard is the contract: a hallucinated empty call cannot drop the entire MemPalace.
