# Knowledge family

> **Status:** audited (D.12).
> See [`README.md`](README.md) for the index and drift policy.
> Tool name list pinned in [`captain_runtime::captain_docs::KNOWLEDGE_FAMILY_TOOLS`](../../crates/captain-runtime/src/captain_docs.rs).

## Tools

The knowledge graph is a **structured** complement to the diary-style triples covered in D.6 (memory). Where `memory_save` records a single fact and lets BM25 + reflection do the heavy lifting, the KG is a typed graph: nodes are entities with attributes, edges are typed relations, and queries can traverse both. Captain reaches for the KG when it needs structure (joining a project to its repos, mapping users to their teams) and for memory when a flat fact is enough.

### `knowledge_add_entity`

Insert or update an entity in the KG. Idempotent on `(type, name)`.

| Field | Required | Notes |
|---|---|---|
| `entity_type` | yes | `User`, `Project`, `Repo`, `Service`, `Server`, …. Free-form but should match an existing type for joinability. |
| `name` | yes | Canonical name (the natural key together with `type`). |
| `attributes` | no | JSON object of typed properties (`{owner: "team-a", url: "https://…"}`). |

Returns the entity id (UUID). Subsequent calls with the same `(type, name)` update attributes in place.

### `knowledge_add_relation`

Insert a typed edge between two entities. Idempotent on `(source, type, target)`.

| Field | Required | Notes |
|---|---|---|
| `source_id` | yes | Entity id from `knowledge_add_entity`. |
| `target_id` | yes | Entity id from `knowledge_add_entity`. |
| `relation_type` | yes | `owns`, `depends_on`, `runs_on`, `member_of`, … |
| `attributes` | no | Optional JSON metadata (`{since: "2026-01-01"}`). |

Returns the edge id.

### `knowledge_query`

Query the graph. Three flavours selected by the shape of the input:

| Field | Required | Notes |
|---|---|---|
| `entity_type` | optional | Filter to a type. |
| `name` | optional | Look up by canonical name. |
| `relation_type` | optional | Traverse only this relation. |
| `traversal_depth` | optional | Hops from the seed entity (default 1, max 4). |
| `query` | optional | Free-text BM25 over name + attributes when no structured filter is supplied. |

Returns a list of `{id, type, name, attributes}` plus, when traversal is requested, the connecting edges.

When the input has zero structured fields, the query falls back to BM25 over MemPalace **plus** the KG, giving Captain a single "rappelle-moi tout ce que tu sais sur X" entry point.

## Sandbox

- **Same SSOT as memory** — KG entities and relations live in the same MemPalace SQLite under `~/.captain/mempalace/`. The path is in Captain's allowed root; ordinary agents cannot read it directly.
- **No PII filter on attributes** — unlike `memory_save`, the KG accepts attribute strings without scanning. The KG is meant for structured operational data; the LLM is responsible for not stuffing credentials into entity attributes (use `secret_*` instead).
- **Cross-agent visibility** — every agent shares the KG. Use entity_type / name namespacing (`Project:foo`, `User:team-a`) to scope a fact to its owner.
- **Append-only by default** — there is **no `knowledge_forget`** today. Mutating an entity is allowed (re-add overwrites attributes); deleting an entity or edge requires a manual SQL pass via `shell_exec` against the MemPalace DB.

## Limites

- `knowledge_query` traversal depth is capped at 4. Deeper analyses must compose multiple shallow queries.
- The free-text fallback is BM25, not embedding-based — synonym matches are weak. For semantic search use `memory_recall` or build the entity attribute strings explicitly.
- Concurrency is single-writer at the SQLite level. Bulk inserts from multiple agents serialize through the kernel's lock; pipelining hundreds of inserts in a tight loop will appear slow.
- Attribute size: keep individual attribute strings under 64 KB. Larger blobs (text bodies, scraped pages) belong in the workspace as files; reference them by path in the entity attributes.
- The KG does not version edges — re-asserting a relation overwrites its `attributes` map but does not record the previous values. Use `memory_save` with `category=event` if you need a timestamped audit trail.
- There is no rename — to change an entity's canonical name, add a new entity, re-attach the relations, and let the old entity dangle (a future `knowledge_forget` will clean it up).

## Exemples

### Golden path — model a project + repo + owner

```
1. knowledge_add_entity({"entity_type":"User","name":"team-a","attributes":{"role":"owner"}})
   → {"id":"u-...", "type":"User"}
2. knowledge_add_entity({"entity_type":"Project","name":"example-service","attributes":{"phase":"production"}})
   → {"id":"p-..."}
3. knowledge_add_entity({"entity_type":"Repo","name":"github.com/org/example-service"})
   → {"id":"r-..."}
4. knowledge_add_relation({"source_id":"u-...","target_id":"p-...","relation_type":"owns"})
5. knowledge_add_relation({"source_id":"p-...","target_id":"r-...","relation_type":"hosted_at"})

knowledge_query({"name":"team-a","relation_type":"owns","traversal_depth":2})
→ traversal: User team-a → Project example-service → Repo github.com/org/example-service
```

### Error case — traversal too deep

```
knowledge_query({"name":"team-a","traversal_depth":7})
→ Err("traversal_depth=7 exceeds max=4 — split into multiple queries").
```

The cap is the contract: deep walks bloat both the response and the SQL plan.
