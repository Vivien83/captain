# hora-graph-core — Developer Guide

A pure-Rust, zero-dependency, bio-inspired knowledge graph engine with bi-temporal facts,
BM25 full-text search, ACT-R memory activation, reconsolidation, dark nodes, FSRS scheduling,
and a dream cycle for memory consolidation.

---

## 1. Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
hora-graph-core = { path = "../hora-graph-core" }
```

Or from a git source:

```toml
[dependencies]
hora-graph-core = { git = "https://github.com/Vivien83/hora-graph-core.git", tag = "v1.0.0" }
```

## 2. Quick Start

```rust
use hora_graph_core::{HoraCore, HoraConfig, props};

fn main() -> hora_graph_core::Result<()> {
    // Create an in-memory graph
    let mut hora = HoraCore::new(HoraConfig::default())?;

    // Add entities
    let alice = hora.add_entity("person", "Alice", Some(props! { "role" => "engineer" }), None)?;
    let rust  = hora.add_entity("language", "Rust", None, None)?;

    // Add a fact (directed edge)
    let _fact = hora.add_fact(alice, rust, "uses", "Alice uses Rust daily", None)?;

    // Search
    let hits = hora.text_search("Rust", 5)?;
    println!("Found {} results", hits.len());

    // Persist to file
    let mut hora = HoraCore::open("my_graph.hora", HoraConfig::default())?;
    let _id = hora.add_entity("demo", "hello", None, None)?;
    hora.flush()?;

    Ok(())
}
```

## 3. Concepts

### Entities

An entity is a node in the graph. It has:

| Field | Type | Description |
|-------|------|-------------|
| `id` | `EntityId(u64)` | Auto-generated unique ID |
| `entity_type` | `String` | Semantic label (e.g. `"person"`, `"concept"`) |
| `name` | `String` | Human-readable name |
| `properties` | `HashMap<String, PropertyValue>` | Arbitrary key-value metadata |
| `embedding` | `Option<Vec<f32>>` | Optional vector for similarity search |
| `created_at` | `i64` | Unix timestamp in ms (system time) |

### Facts (Edges)

A fact is a directed, bi-temporal relation between two entities:

| Field | Type | Description |
|-------|------|-------------|
| `id` | `EdgeId(u64)` | Auto-generated unique ID |
| `source` | `EntityId` | Origin entity |
| `target` | `EntityId` | Destination entity |
| `relation_type` | `String` | Label (e.g. `"knows"`, `"works_at"`) |
| `description` | `String` | Human-readable description |
| `confidence` | `f32` | Confidence score `[0.0, 1.0]` |
| `valid_at` | `i64` | When this fact became true (world time) |
| `invalid_at` | `i64` | When it ceased to be true; `0` = still valid |
| `created_at` | `i64` | When it was stored (system time) |

### Bi-temporal Model

Every fact has two time axes:
- **World time** (`valid_at` / `invalid_at`): when the fact is true in reality
- **System time** (`created_at`): when the fact was recorded

Use `invalidate_fact()` for soft-delete (sets `invalid_at` to now). Use `delete_fact()` for hard-delete.

### Episodes

An episode is a snapshot of an interaction — a group of related entity and fact IDs with a source and session:

| Field | Type | Description |
|-------|------|-------------|
| `id` | `u64` | Auto-generated unique ID |
| `source` | `EpisodeSource` | `Conversation`, `Document`, or `Api` |
| `session_id` | `String` | Groups episodes from the same session |
| `entity_ids` | `Vec<EntityId>` | Entities referenced in this episode |
| `fact_ids` | `Vec<EdgeId>` | Facts referenced in this episode |
| `consolidation_count` | `u32` | How many times this episode has been consolidated |

### Property Values

```rust
use hora_graph_core::PropertyValue;

// Supported types
PropertyValue::String("hello".into())
PropertyValue::Int(42)
PropertyValue::Float(3.14)
PropertyValue::Bool(true)

// Convenient construction with props! macro
use hora_graph_core::props;
let p = props! {
    "name" => "Alice",
    "age" => 30,
    "active" => true,
};
```

## 4. CRUD Operations

### Entities

```rust
use hora_graph_core::{HoraCore, HoraConfig, EntityUpdate, props};

let mut hora = HoraCore::new(HoraConfig::default())?;

// Create
let id = hora.add_entity("person", "Alice",
    Some(props! { "email" => "alice@example.com" }),
    None, // no embedding
)?;

// Read
let entity = hora.get_entity(id)?.expect("exists");
assert_eq!(entity.name, "Alice");

// Update (partial — only Some fields are changed)
hora.update_entity(id, EntityUpdate {
    name: Some("Alice Smith".into()),
    properties: Some(props! { "email" => "alice@smith.com", "phone" => "555-0100" }),
    ..Default::default()
})?;

// Delete (cascades: removes all connected edges)
hora.delete_entity(id)?;
```

### Facts

```rust
let alice = hora.add_entity("person", "Alice", None, None)?;
let bob   = hora.add_entity("person", "Bob", None, None)?;

// Create a fact
let fact_id = hora.add_fact(alice, bob, "knows", "Met at RustConf 2025", Some(0.9))?;

// Read
let fact = hora.get_fact(fact_id)?.expect("exists");
assert_eq!(fact.relation_type, "knows");

// Get all facts for an entity (both directions)
let facts = hora.get_entity_facts(alice)?;

// Update a fact (partial — only Some fields are changed)
use hora_graph_core::FactUpdate;
hora.update_fact(fact_id, FactUpdate {
    confidence: Some(0.95),
    description: Some("Met at RustConf 2025 — became close friends".into()),
})?;

// Soft-delete (bi-temporal: marks invalid_at = now)
hora.invalidate_fact(fact_id)?;

// Hard-delete
// hora.delete_fact(fact_id)?;
```

### Episodes

```rust
use hora_graph_core::EpisodeSource;

let ep_id = hora.add_episode(
    EpisodeSource::Conversation,
    "session-001",
    &[alice, bob],      // entity IDs
    &[fact_id],         // fact IDs
)?;

// Read
let episode = hora.get_episode(ep_id)?.expect("exists");

// Filter
let episodes = hora.get_episodes(
    Some("session-001"), // session_id filter
    None,                // source filter
    None,                // since (epoch ms)
    None,                // until (epoch ms)
)?;

// Increment consolidation count (used by dream cycle's replay step)
hora.increment_consolidation(ep_id)?;
```

## 5. Search

### BM25 Full-Text Search

Searches over entity names and all string properties. Uses BM25+ scoring.

```rust
let _id = hora.add_entity("doc", "Rust Programming",
    Some(props! { "body" => "Rust is a systems programming language" }),
    None,
)?;

let hits = hora.text_search("programming", 10)?;
for hit in &hits {
    println!("entity:{} score={:.3}", hit.entity_id.0, hit.score);
}
```

### Vector Search

Brute-force cosine similarity over entity embeddings. Requires `embedding_dims > 0`.

```rust
use hora_graph_core::HoraConfig;

let config = HoraConfig { embedding_dims: 3, ..Default::default() };
let mut hora = HoraCore::new(config)?;

let _id = hora.add_entity("vec", "test", None, Some(&[1.0, 0.0, 0.0]))?;
let _id = hora.add_entity("vec", "similar", None, Some(&[0.9, 0.1, 0.0]))?;

let hits = hora.vector_search(&[1.0, 0.0, 0.0], 5)?;
// hits[0] will be "test" (exact match, score ≈ 1.0)
```

### Hybrid Search (BM25 + Vector via RRF)

Combines both legs using Reciprocal Rank Fusion. Provide text, embedding, or both.

```rust
use hora_graph_core::SearchOpts;

let hits = hora.search(
    Some("programming"),       // BM25 query
    Some(&[1.0, 0.0, 0.0]),   // vector query
    SearchOpts { top_k: 10, include_dark: false },
)?;
```

If only one leg is provided, the other is skipped. Returns empty if neither is provided.

## 6. Traversal

### BFS Traversal

```rust
use hora_graph_core::TraverseOpts;

// Traverse up to depth 3 from Alice
let result = hora.traverse(alice, TraverseOpts { depth: 3 })?;
println!("Discovered {} entities, {} edges",
    result.entity_ids.len(), result.edge_ids.len());
```

### Neighbors

```rust
// Direct neighbors only (depth 1)
let neighbor_ids = hora.neighbors(alice)?;
```

### Timeline

```rust
// All facts involving Alice, sorted by valid_at
let timeline = hora.timeline(alice)?;
for fact in &timeline {
    println!("{}: {} -> {}", fact.relation_type, fact.source.0, fact.target.0);
}
```

### Temporal Query

```rust
// All facts valid at a specific point in time
let timestamp = 1700000000000_i64; // epoch ms
let valid_facts = hora.facts_at(timestamp)?;
```

## 7. Bio-Inspired Memory

### ACT-R Activation

Every entity has an activation score based on the ACT-R Base-Level Learning equation.
It decays over time and increases with each access.

```rust
// get_entity() automatically records an access
let _ = hora.get_entity(alice)?;

// Get current activation score
let activation = hora.get_activation(alice);
// Some(f64) — higher = more active, NEG_INFINITY = never accessed

// Manually record an access (called automatically by get_entity/search)
hora.record_access(alice);
```

The activation formula is: `B_i = ln(Σ t_j^(-d))` where `t_j` is time since each access
and `d = 0.5` (decay rate).

### Reconsolidation

When a memory is reactivated, it enters a labile (destabilized) phase before restabilizing.
This models the reconsolidation window from neuroscience.

```rust
use hora_graph_core::MemoryPhase;

let phase = hora.get_memory_phase(alice);
// Some(&MemoryPhase::Stable) — normal state
// Some(&MemoryPhase::Labile { .. }) — destabilized, can be updated
// Some(&MemoryPhase::Restabilizing { .. }) — being restabilized
// Some(&MemoryPhase::Dark { .. }) — silenced (below threshold)

let multiplier = hora.get_stability_multiplier(alice);
// Increases by 1.2× each completed reconsolidation cycle
```

### Dark Nodes

Entities with very low activation that haven't been accessed recently become "dark" — hidden from search results but not deleted.

```rust
// Run a dark node pass (marks low-activation entities as Dark)
let count = hora.dark_node_pass();
println!("{} entities darkened", count);

// List all dark entities
let dark_ids = hora.dark_nodes();

// Recover a dark entity (transitions to Labile for re-encoding)
let recovered = hora.attempt_recovery(alice);

// List entities eligible for garbage collection
let gc_candidates = hora.gc_candidates();
```

### FSRS Scheduling

Free Spaced Repetition Scheduler — optimizes review intervals.

```rust
// Current retrievability (0.0 to 1.0, decays over time)
let r = hora.get_retrievability(alice);

// Optimal next review interval in days
let days = hora.get_next_review_days(alice);

// Current stability in days
let stability = hora.get_fsrs_stability(alice);
```

### Spreading Activation

ACT-R spreading activation through the graph with fan effect.

```rust
use hora_graph_core::SpreadingParams;

let sources = vec![(alice, 1.0)]; // source entities with initial activation
let result = hora.spread_activation(&sources, &SpreadingParams::default())?;

for (entity_id, activation) in &result {
    println!("entity:{} spread_activation={:.3}", entity_id.0, activation);
}
```

## 8. Dream Cycle

The dream cycle is a 6-step consolidation pipeline inspired by sleep neuroscience:

| Step | What it does | Inspiration |
|------|-------------|-------------|
| **SHY** | Reduce all activation scores by factor (default 0.78) | Tononi & Cirelli 2003 |
| **Replay** | Reactivate entities from mixed recent/old episodes | McClelland 1995, Ji & Wilson 2007 |
| **CLS** | Extract recurring episodic patterns into semantic facts | Kumaran et al. 2016 |
| **Linking** | Create temporal links between co-created entities | Zeithamova & Preston 2010 |
| **Dark check** | Silence low-activation entities | Inhibitory consolidation |
| **GC** | Delete GC-eligible dark entities (opt-in) | Memory decay |

```rust
use hora_graph_core::DreamCycleConfig;

// Run with all steps (GC disabled by default)
let stats = hora.dream_cycle(&DreamCycleConfig::default())?;
println!("Downscaled: {}", stats.entities_downscaled);
println!("Replayed: {} episodes", stats.replay.episodes_replayed);
println!("CLS: {} facts created", stats.cls.facts_created);
println!("Links: {} created", stats.linking.links_created);
println!("Darkened: {}", stats.dark_nodes_marked);

// Run with GC enabled
let stats = hora.dream_cycle(&DreamCycleConfig {
    gc: true,
    ..Default::default()
})?;
println!("GC deleted: {}", stats.gc_deleted);

// Run only specific steps
let stats = hora.dream_cycle(&DreamCycleConfig {
    shy: true,
    replay: false,
    cls: false,
    linking: false,
    dark_check: true,
    gc: false,
})?;
```

### Individual Steps

Each step can also be called independently:

```rust
// SHY downscaling (factor: 0.0–1.0)
let count = hora.shy_downscaling(0.78);

// Interleaved replay
let replay_stats = hora.interleaved_replay()?;

// CLS transfer
let cls_stats = hora.cls_transfer()?;

// Memory linking
let link_stats = hora.memory_linking()?;
```

## 9. Persistence

### File-backed Instance

```rust
use hora_graph_core::{HoraCore, HoraConfig};

// Open (creates file if it doesn't exist, loads if it does)
let mut hora = HoraCore::open("data.hora", HoraConfig::default())?;

// Write operations...
let _id = hora.add_entity("demo", "test", None, None)?;

// Flush to disk (atomic: writes to .tmp then renames)
hora.flush()?;
```

### Snapshots

```rust
// Copy current state to a separate file
hora.snapshot("backup.hora")?;
```

### File Verification

```rust
use hora_graph_core::verify_file;

let report = verify_file("data.hora")?;
println!("Entities: {}", report.entity_count);
println!("Edges: {}", report.edge_count);
println!("Episodes: {}", report.episode_count);
```

### In-Memory Only

```rust
let mut hora = HoraCore::new(HoraConfig::default())?;
// flush() will return an error — no file path configured
// Use snapshot() to write to a specific file if needed
```

## 10. Configuration Reference

### HoraConfig

```rust
use hora_graph_core::{HoraConfig, DedupConfig};

let config = HoraConfig {
    // Embedding vector dimensions. 0 = text-only mode (no vector search).
    embedding_dims: 384,

    // Deduplication settings
    dedup: DedupConfig {
        enabled: true,             // Enable dedup on add_entity
        name_exact: true,          // Detect normalized name matches
        jaccard_threshold: 0.85,   // Token overlap threshold (0.0 = disabled)
        cosine_threshold: 0.92,    // Embedding similarity threshold (0.0 = disabled)
    },
};

// Disable deduplication
let config = HoraConfig {
    dedup: DedupConfig::disabled(),
    ..Default::default()
};
```

### ConsolidationParams (Dream Cycle)

These are set internally via `ConsolidationParams::default()`:

| Parameter | Default | Description |
|-----------|---------|-------------|
| `shy_factor` | `0.78` | Multiplicative factor for SHY downscaling |
| `recent_ratio` | `0.7` | Fraction of recent episodes in replay (70%) |
| `max_replay_items` | `100` | Max episodes per replay cycle |
| `cls_threshold` | `3` | Min consolidation_count for CLS eligibility |
| `linking_window_ms` | `21_600_000` | Temporal window for linking (6 hours) |
| `linking_max_neighbors` | `20` | Max temporal neighbors per entity |

### DreamCycleConfig

| Field | Default | Description |
|-------|---------|-------------|
| `shy` | `true` | Enable SHY downscaling |
| `replay` | `true` | Enable interleaved replay |
| `cls` | `true` | Enable CLS semantic transfer |
| `linking` | `true` | Enable temporal memory linking |
| `dark_check` | `true` | Enable dark node detection |
| `gc` | `false` | Enable GC of dark entities (destructive) |

### DarkNodeParams

| Parameter | Default | Description |
|-----------|---------|-------------|
| `silencing_threshold` | `-2.0` | Activation below which entities go dark |
| `silencing_delay_secs` | `604_800` | Min seconds since last access (7 days) |
| `gc_eligible_after_secs` | `2_592_000` | Seconds as dark before GC eligible (30 days) |

### FsrsParams

| Parameter | Default | Description |
|-----------|---------|-------------|
| `desired_retention` | `0.9` | Target retrievability for scheduling |
| `initial_stability_days` | `1.0` | Initial stability for new entities |
| `decay` | `0.2` | Power-law decay exponent |

### SpreadingParams

| Parameter | Default | Description |
|-----------|---------|-------------|
| `max_depth` | `3` | Maximum hops for spreading |
| `s_max` | `1.6` | Maximum association strength |
| `decay_per_hop` | `0.5` | Multiplicative decay per hop |

### ReconsolidationParams

| Parameter | Default | Description |
|-----------|---------|-------------|
| `labile_duration_secs` | `3600.0` | Duration of labile phase (1 hour) |
| `restabilization_duration_secs` | `7200.0` | Duration of restabilization (2 hours) |
| `reactivation_threshold` | `-1.0` | Activation level that triggers reconsolidation |
| `restabilization_boost` | `1.2` | Stability multiplier gained per cycle |

## Stats

```rust
let stats = hora.stats()?;
println!("Entities: {}, Edges: {}, Episodes: {}",
    stats.entities, stats.edges, stats.episodes);
```

## Error Handling

All fallible operations return `hora_graph_core::Result<T>`, which wraps `HoraError`:

```rust
use hora_graph_core::HoraError;

match hora.get_entity(EntityId(999)) {
    Ok(Some(e)) => println!("Found: {}", e.name),
    Ok(None) => println!("Not found"),
    Err(HoraError::Io(e)) => eprintln!("I/O error: {}", e),
    Err(e) => eprintln!("Error: {}", e),
}
```

Error variants: `Io`, `CorruptedFile`, `InvalidFile`, `VersionMismatch`, `EntityNotFound`,
`EdgeNotFound`, `DimensionMismatch`, `AlreadyInvalidated`, `StringTooLong`, `StorageFull`.

---

## 11. Language Bindings

hora-graph-core exposes the same API across Rust, Node.js, Python, WASM, and C.

### Node.js (napi-rs)

```js
const { HoraCore } = require('hora-graph-core');

// In-memory
const g = HoraCore.newMemory();

// File-backed
const g2 = HoraCore.open('data.hora');

// IDs are u32 in JS (u64 internally)
const alice = g.addEntity('person', 'Alice', { role: 'engineer' });
const bob   = g.addEntity('person', 'Bob');

const factId = g.addFact(alice, bob, 'knows', 'Met at RustConf', 0.9);

// Traverse
const result = g.traverse(alice, { depth: 3 });
// result.entityIds: number[], result.edgeIds: number[]

// Search
const hits = g.textSearch('Alice', 5);
// hits: [{ entityId, name, entityType, score }]

// Activation
const activation = g.getActivation(alice);

// Memory phase
const phase = g.getMemoryPhase(alice); // "stable" | "labile" | "restabilizing" | "dark" | null

// Persistence
g.flush();
g.snapshot('backup.hora');

// Stats
const stats = g.stats();
// { entities, edges, episodes }
```

### Python (PyO3)

```python
from hora_graph_core import HoraGraph

# In-memory
g = HoraGraph()

# File-backed
g = HoraGraph(path="data.hora")

# IDs are int (u64)
alice = g.add_entity("person", "Alice", properties={"role": "engineer"})
bob   = g.add_entity("person", "Bob")

fact_id = g.add_fact(alice, bob, "knows", "Met at RustConf", 0.9)

# Traverse
result = g.traverse(alice, depth=3)
# result = {"entity_ids": [...], "edge_ids": [...]}

# Search
hits = g.text_search("Alice", k=5)
# hits = [{"entity_id": ..., "name": ..., "entity_type": ..., "score": ...}]

# Memory features
activation = g.get_activation(alice)
phase = g.get_memory_phase(alice)
r = g.get_retrievability(alice)

# Dream cycle
stats = g.dream_cycle()

# Persistence
g.flush()
g.snapshot("backup.hora")
```

### WASM (wasm-bindgen)

```js
import init, { HoraCore } from 'hora-graph-wasm';

await init();
const g = HoraCore.newMemory();

// Same API as Node.js, but memory-only (no file persistence)
const alice = g.addEntity('person', 'Alice', { role: 'engineer' });
const bob   = g.addEntity('person', 'Bob');
g.addFact(alice, bob, 'knows', 'Met at RustConf', 0.9);

const result = g.traverse(alice, 3);
const stats = g.stats();
```

### C FFI (cbindgen)

```c
#include "hora_graph_core.h"

HoraCore* g = hora_new_memory(0);
uint64_t alice = hora_add_entity(g, "person", "Alice", NULL);
uint64_t bob   = hora_add_entity(g, "person", "Bob", NULL);
hora_add_fact(g, alice, bob, "knows", "Met at RustConf", 0.9f);

hora_flush(g, "data.hora");
hora_free(g);
```

### API Parity Table

| Method | Rust | Node.js | Python | WASM | C |
|--------|------|---------|--------|------|---|
| add_entity | `add_entity()` | `addEntity()` | `add_entity()` | `addEntity()` | `hora_add_entity()` |
| get_entity | `get_entity()` | `getEntity()` | `get_entity()` | `getEntity()` | `hora_get_entity()` |
| add_fact | `add_fact()` | `addFact()` | `add_fact()` | `addFact()` | `hora_add_fact()` |
| traverse | `traverse()` | `traverse()` | `traverse()` | `traverse()` | `hora_traverse()` |
| text_search | `text_search()` | `textSearch()` | `text_search()` | `search()` | `hora_text_search()` |
| dream_cycle | `dream_cycle()` | `dreamCycle()` | `dream_cycle()` | — | — |
| flush | `flush()` | `flush()` | `flush()` | — | `hora_flush()` |

## 12. Complete API Reference

### HoraCore Methods

| Category | Method | Description |
|----------|--------|-------------|
| **Create** | `new(config)` | New in-memory instance |
| | `open(path, config)` | Open file-backed instance |
| **Entities** | `add_entity(type, name, props, embedding)` | Create entity, returns `EntityId` |
| | `get_entity(id)` | Read entity (records access for activation) |
| | `update_entity(id, update)` | Partial update (`EntityUpdate`) |
| | `delete_entity(id)` | Delete + cascade edges |
| **Facts** | `add_fact(source, target, relation, desc, confidence)` | Create directed edge |
| | `get_fact(id)` | Read fact |
| | `update_fact(id, update)` | Partial update (`FactUpdate`: confidence, description) |
| | `invalidate_fact(id)` | Bi-temporal soft-delete (sets `invalid_at`) |
| | `delete_fact(id)` | Physical hard-delete |
| | `get_entity_facts(entity_id)` | All facts for an entity |
| **Traversal** | `traverse(start, opts)` | BFS up to depth, returns entity+edge IDs |
| | `neighbors(entity_id)` | Direct neighbor IDs |
| | `timeline(entity_id)` | Facts sorted by `valid_at` |
| | `facts_at(timestamp)` | All facts valid at time `t` |
| **Search** | `text_search(query, k)` | BM25+ full-text search |
| | `vector_search(query, k)` | SIMD cosine similarity |
| | `search(text, embedding, opts)` | Hybrid RRF (BM25 + vector) |
| **Memory** | `get_activation(id)` | ACT-R base-level activation |
| | `record_access(id)` | Manually record access event |
| | `get_memory_phase(id)` | Stable/Labile/Restabilizing/Dark |
| | `get_stability_multiplier(id)` | Reconsolidation stability gain |
| | `get_retrievability(id)` | FSRS retrievability (0.0–1.0) |
| | `get_next_review_days(id)` | Optimal review interval |
| | `get_fsrs_stability(id)` | FSRS stability in days |
| | `spread_activation(sources, params)` | ACT-R spreading with fan effect |
| **Dark Nodes** | `dark_node_pass()` | Silence low-activation entities |
| | `dark_nodes()` | List dark entity IDs |
| | `attempt_recovery(id)` | Recover dark entity |
| | `gc_candidates()` | Dark entities eligible for deletion |
| **Consolidation** | `shy_downscaling(factor)` | Reduce all activations |
| | `interleaved_replay()` | Replay recent+old episodes |
| | `cls_transfer()` | Extract semantic facts from episodes |
| | `memory_linking()` | Create temporal co-occurrence links |
| | `dream_cycle(config)` | Run full 6-step consolidation |
| **Episodes** | `add_episode(source, session, entities, facts)` | Create episode |
| | `get_episode(id)` | Read episode |
| | `get_episodes(session, source, since, until)` | Filter episodes |
| | `increment_consolidation(id)` | Bump consolidation count |
| **Persistence** | `flush()` | Save to configured file path |
| | `snapshot(dest)` | Copy to a different file |
| | `verify_file(path)` | Validate file integrity |
| **Stats** | `stats()` | Entity/edge/episode counts |

---

**hora-graph-core v1.0.0** — [github.com/Vivien83/hora-graph-core](https://github.com/Vivien83/hora-graph-core)
